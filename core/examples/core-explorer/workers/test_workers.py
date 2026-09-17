"""Offline tests: no native binding, credentials, sockets or services."""
import asyncio
import contextlib
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import run as worker

DOMAIN = "11111111-1111-4111-8111-111111111111"
INSTALL = "22222222-2222-4222-8222-222222222222"
CLIENT = "33333333-3333-4333-8333-333333333333"
INPUT = "44444444-4444-4444-8444-444444444444"
TASK = "55555555-5555-4555-8555-555555555555"
OUTPUT = "66666666-6666-4666-8666-666666666666"


def settings(role="compute"):
    result = dict(role=role, domain_id=DOMAIN, installation_id=INSTALL,
                  client_id=CLIENT, dds_url="https://dds.example.invalid",
                  dms_url="https://dms.example.invalid/v1", registration="synthetic-only")
    result.update(wallet_key="1" * 64) if role == "compute" else result.update(
        audience="https://dds.example.invalid/robots")
    return result


class ConfigTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.path = Path(self.directory.name) / "worker.json"
        self.path.write_text(json.dumps(settings()))
        self.path.chmod(0o600)

    def test_valid_private_config_and_stable_identity(self):
        first = worker.load_config(str(self.path))
        self.assertEqual(first, worker.load_config(str(self.path)))
        self.assertEqual(first.client_id, CLIENT)
        self.assertEqual(first.capability, f"/examples/compute-robot/{INSTALL}/compute/v1")
        self.assertNotIn("synthetic-only", repr(first))
        self.assertNotIn("1" * 64, repr(first))
        self.assertEqual(worker.validate_config(settings("robot")).role, "robot")

    def test_reject_invalid_schema_and_values(self):
        for key, value in [("role", "user"), ("client_id", "new-on-start"),
                           ("domain_id", None), ("installation_id", "0" * 32),
                           ("wallet_key", "bad"), ("registration", "<replace>"),
                           ("wallet_key", "0" * 64), ("wallet_key", "f" * 64),
                           ("registration", "secret\n"), ("unknown", "secret")]:
            with self.subTest(key=key, value=value), self.assertRaises(worker.InvalidConfig):
                worker.validate_config({**settings(), key: value})
        for key in settings():
            config = settings()
            del config[key]
            with self.subTest(missing=key), self.assertRaises(worker.InvalidConfig):
                worker.validate_config(config)
        with self.assertRaises(worker.InvalidConfig):
            worker.validate_config({**settings("robot"), "wallet_key": "1" * 64})

    def test_reject_unsafe_urls(self):
        for url in ["http://dds.example.invalid", "https://user:secret@example.invalid",
                    "https://example.invalid/?token=x", "https://example.invalid/#x",
                    "https://example.invalid:bad", "https://example.invalid:0",
                    "https://example.invalid\n", "file:///tmp/file", "https://<replace>",
                    "http://localhost", "https://example.invalid\\@evil.invalid"]:
            with self.subTest(url=url), self.assertRaises(worker.InvalidConfig):
                worker.validate_config({**settings(), "dds_url": url})
        worker.validate_config({**settings(), "dds_url": "http://127.0.0.1:1234"})

    def test_reject_file_and_parent_permissions(self):
        for mode in (0o644, 0o640, 0o400, 0o700):
            self.path.chmod(mode)
            with self.assertRaises(worker.InvalidConfig):
                worker.load_config(str(self.path))
        self.path.chmod(0o600)
        Path(self.directory.name).chmod(0o755)
        with self.assertRaises(worker.InvalidConfig):
            worker.load_config(str(self.path))
        Path(self.directory.name).chmod(0o700)

    def test_reject_symlinks_hardlinks_and_non_regular_files(self):
        link = self.path.with_name("link")
        link.symlink_to(self.path)
        with self.assertRaises(worker.InvalidConfig):
            worker.load_config(str(link))
        link.unlink()
        os.link(self.path, link)
        with self.assertRaises(worker.InvalidConfig):
            worker.load_config(str(self.path))
        link.unlink()
        self.path.unlink()
        os.mkfifo(self.path, 0o600)
        with self.assertRaises(worker.InvalidConfig):
            worker.load_config(str(self.path))

    def test_reject_symlink_directory(self):
        with tempfile.TemporaryDirectory() as parent:
            link = Path(parent) / "private"
            link.symlink_to(self.directory.name, target_is_directory=True)
            with self.assertRaises(worker.InvalidConfig):
                worker.load_config(str(link / "worker.json"))

    def test_reject_wrong_owner(self):
        with patch.object(worker.os, "getuid", return_value=os.getuid() + 1):
            with self.assertRaises(worker.InvalidConfig):
                worker.load_config(str(self.path))

    def test_reject_duplicate_oversized_and_malformed_json(self):
        for raw in ['{"role":"compute","role":"robot"}', "[", "x" * (worker.MAX_CONFIG + 1),
                    '"' + "[" * 1000, 'null', '[1]']:
            self.path.write_text(raw)
            with self.subTest(raw=raw[:20]), self.assertRaises(worker.InvalidConfig):
                worker.load_config(str(self.path))

    def test_check_does_not_import_sdk_or_contact_network(self):
        code = '''import run, sys
class Block:
    def find_spec(self, fullname, *args):
        if fullname == 'auki_sdk': raise AssertionError('SDK imported')
sys.meta_path.insert(0, Block())
def audit(event, args):
    if event.startswith('socket.'): raise AssertionError('network attempted')
sys.addaudithook(audit)
raise SystemExit(run.main(['--check', '--config', sys.argv[1]]))
'''
        result = subprocess.run([sys.executable, "-B", "-c", code, str(self.path)],
                                cwd=Path(__file__).parent, capture_output=True, text=True, timeout=5)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, '{"event":"configuration_valid"}\n')

    def test_error_logs_are_fixed_and_redacted(self):
        self.path.write_text('{"private":"do-not-log"')
        output = io.StringIO()
        with contextlib.redirect_stderr(output):
            self.assertEqual(worker.main(["--check", "--config", str(self.path)]), 2)
        self.assertEqual(output.getvalue(), '{"event":"configuration_rejected"}\n')


class Data:
    def __init__(self, content=b"hello, robot!"):
        self.content, self.writes, self.closed = content, [], False

    async def read_to(self, data_id, sink, **limits):
        self.limits = limits
        await sink(self.content)

    async def write(self, content, **options):
        self.writes.append((content, options))
        return {"id": OUTPUT}

    async def close(self):
        await asyncio.sleep(0)
        self.closed = True


class Task:
    def __init__(self, config, data=None):
        self.id, self.domain_id, self.capability = TASK, DOMAIN, config.capability
        self.meta, self.inputs_cids = {"input_id": INPUT, "run_id": INSTALL}, [INPUT]
        self.storage, self.data_calls, self.events = data or Data(), 0, []

    def data(self):
        self.data_calls += 1
        return self.storage

    async def progress(self, value):
        self.events.append(value)

    async def log_event(self, value):
        self.events.append(value)


class HandlerTests(unittest.IsolatedAsyncioTestCase):
    async def test_both_upstream_handlers_and_receipts(self):
        for role in ("compute", "robot"):
            config = worker.validate_config(settings(role))
            task = Task(config)
            result = await worker.make_handler(config)(task)
            content, options = task.storage.writes[0]
            self.assertEqual(options["name"], f"sdk-{INSTALL}-{role}-{TASK}")
            self.assertEqual(result["output_cids"], [OUTPUT])
            receipt_bytes = content if role == "compute" else task.storage.content
            self.assertEqual(result["meta"], dict(data_id=OUTPUT, run_id=INSTALL,
                sha256=hashlib.sha256(receipt_bytes).hexdigest(), bytes=len(receipt_bytes)))
            self.assertEqual(task.storage.limits, dict(max_bytes=65536, max_chunk_bytes=16384))
            self.assertTrue(task.storage.closed)
            self.assertEqual(len(task.events), 4)
            if role == "compute":
                self.assertEqual(content, b"HELLO, ROBOT!")
                self.assertEqual(result["meta"]["sha256"], hashlib.sha256(content).hexdigest())
            else:
                self.assertEqual(json.loads(content), dict(input_id=INPUT, bytes=13,
                    sha256=hashlib.sha256(b"hello, robot!").hexdigest()))

    async def test_scope_rejected_before_data_io(self):
        config = worker.validate_config(settings())
        for key, value in [("domain_id", CLIENT), ("capability", "other"), ("id", "bad"),
                           ("inputs_cids", []), ("meta", {"input_id": INPUT, "run_id": CLIENT}),
                           ("meta", {"input_id": "bad", "run_id": INSTALL}),
                           ("meta", {"input_id": INPUT, "run_id": INSTALL, "remote_peer_id": "x"})]:
            task = Task(config)
            setattr(task, key, value)
            with self.subTest(key=key), self.assertRaisesRegex(RuntimeError, "^worker task failed$"):
                await worker.make_handler(config)(task)
            self.assertEqual(task.data_calls, 0)

    async def test_input_output_and_utf8_limits(self):
        config = worker.validate_config(settings())
        for content in (b"x" * 65537, b"\xff", "ΐ".encode() * 21845):
            task = Task(config, Data(content))
            with self.assertRaisesRegex(RuntimeError, "^worker task failed$"):
                await worker.make_handler(config)(task)
            self.assertEqual(task.storage.writes, [])
            self.assertTrue(task.storage.closed)
        task = Task(config, Data(b"a" * 65536))
        await worker.make_handler(config)(task)
        self.assertEqual(len(task.storage.writes[0][0]), 65536)

    async def test_timeout_and_cancellation_drain_read_and_close(self):
        config = worker.validate_config(settings())
        for cancel in (False, True):
            started, drained = asyncio.Event(), asyncio.Event()
            data = Data()
            async def blocked(*args, **kwargs):
                started.set()
                try:
                    await asyncio.Future()
                finally:
                    await asyncio.sleep(0)
                    drained.set()
            data.read_to = blocked
            with patch.object(worker, "TASK_SECONDS", 0.01 if not cancel else 30):
                running = asyncio.create_task(worker.make_handler(config)(Task(config, data)))
                await started.wait()
                if cancel:
                    running.cancel()
                with self.assertRaises(asyncio.CancelledError if cancel else RuntimeError):
                    await running
            self.assertTrue(drained.is_set())
            self.assertTrue(data.closed)

    async def test_write_denial_is_fixed_and_client_closed(self):
        config = worker.validate_config(settings())
        data = Data()
        async def denied(*args, **kwargs):
            raise RuntimeError("private-token-do-not-log")
        data.write = denied
        with self.assertRaisesRegex(RuntimeError, "^worker task failed$"):
            await worker.make_handler(config)(Task(config, data))
        self.assertTrue(data.closed)

    async def test_adapter_rejects_multiple_writes_and_alternate_targets(self):
        data = Data()
        bounded = worker.BoundedData(data, INPUT, "expected", "compute")
        for content, name, data_type in [(b"x" * 65537, "expected", "example.text.v1"),
                                         (b"x", "other", "example.text.v1"),
                                         (b"x", "expected", "other")]:
            with self.assertRaises(ValueError):
                await bounded.write(content, name=name, data_type=data_type)
        self.assertEqual(data.writes, [])
        await bounded.write(b"x", name="expected", data_type="example.text.v1")
        with self.assertRaises(ValueError):
            await bounded.write(b"x", name="expected", data_type="example.text.v1")
        with self.assertRaises(ValueError):
            await bounded.read(OUTPUT)

    async def test_stream_limit_counts_all_chunks(self):
        config = worker.validate_config(settings())
        data = Data()
        async def chunks(data_id, sink, **limits):
            for _ in range(5):
                await sink(b"a" * 16384)
        data.read_to = chunks
        with self.assertRaises(RuntimeError):
            await worker.make_handler(config)(Task(config, data))
        self.assertEqual(data.writes, [])
        self.assertTrue(data.closed)

    async def test_data_close_error_is_fixed(self):
        config = worker.validate_config(settings())
        data = Data()
        async def fail():
            raise RuntimeError("secret-close-error")
        data.close = fail
        with self.assertRaisesRegex(RuntimeError, "^worker task failed$"):
            await worker.make_handler(config)(Task(config, data))


class LifecycleTests(unittest.IsolatedAsyncioTestCase):
    async def exercise(self, role="compute", assigned=DOMAIN, fail_close=False,
                       fail_constructor=False, fail_start=False):
        events, options = [], {}
        ready, stop = asyncio.Event(), asyncio.Event()
        class Credential:
            def __init__(self, **kwargs):
                options.update(kwargs)
            async def assigned_domain_id(self):
                return assigned
            async def close(self):
                await asyncio.sleep(0)
                events.append("credential_closed")
        class Tasks:
            def __init__(self, credential, handlers, **kwargs):
                if fail_constructor:
                    raise RuntimeError("constructor failed")
                options["handlers"] = handlers
            async def start(self):
                events.append("start")
                if fail_start:
                    raise RuntimeError("startup failed")
            async def run(self):
                ready.set()
                try:
                    await asyncio.Future()
                finally:
                    await asyncio.sleep(0)
                    events.append("run_drained")
            async def close(self):
                await asyncio.sleep(0)
                events.append("tasks_closed")
                if fail_close:
                    raise RuntimeError("close failed")
        sdk = SimpleNamespace(AukiComputeCredential=Credential, AukiRobotCredential=Credential,
                              AukiDmsTasks=Tasks)
        running = asyncio.create_task(worker.serve(worker.validate_config(settings(role)), stop, sdk))
        if assigned == DOMAIN and not fail_constructor and not fail_start:
            await asyncio.wait_for(ready.wait(), 1)
            stop.set()
            stop.set()
        if assigned != DOMAIN or fail_close or fail_constructor or fail_start:
            with self.assertRaises(RuntimeError):
                await running
        else:
            await running
        self.assertEqual(events[-1], "credential_closed")
        self.assertEqual(options["client_id"], CLIENT)
        self.assertNotIn("peer_identity_file", options)
        return events, options

    async def test_stop_drains_runtime_then_credential_for_both_roles(self):
        for role in ("compute", "robot"):
            events, options = await self.exercise(role)
            self.assertEqual(events, ["start", "run_drained", "tasks_closed", "credential_closed"])
            self.assertEqual(list(options["handlers"]), [f"/examples/compute-robot/{INSTALL}/{role}/v1"])
            self.assertEqual("wallet_key" in options, role == "compute")

    async def test_wrong_or_unassigned_robot_never_starts_loop(self):
        for assigned in (None, CLIENT):
            events, _ = await self.exercise("robot", assigned)
            self.assertEqual(events, ["tasks_closed", "credential_closed"])

    async def test_credential_close_even_when_runtime_close_fails(self):
        await self.exercise(fail_close=True)

    async def test_credential_close_when_runtime_construction_fails(self):
        events, _ = await self.exercise(fail_constructor=True)
        self.assertEqual(events, ["credential_closed"])

    async def test_startup_failure_closes_both_owners(self):
        events, _ = await self.exercise(fail_start=True)
        self.assertEqual(events, ["start", "tasks_closed", "credential_closed"])


class ProtectionTests(unittest.TestCase):
    def test_linux_protection_before_private_loading(self):
        code = """import ctypes, resource, run, os
run.disable_dumps()
assert resource.getrlimit(resource.RLIMIT_CORE) == (0, 0)
assert ctypes.CDLL(None).prctl(3, 0, 0, 0, 0) == 0
assert os.environ['RUST_LOG'] == 'off'
"""
        result = subprocess.run([sys.executable, "-B", "-c", code],
                                cwd=Path(__file__).parent, capture_output=True, timeout=5)
        self.assertEqual(result.returncode, 0)

    def test_protection_failure_prevents_config_read_and_has_safe_error(self):
        output = io.StringIO()
        with patch.object(worker, "disable_dumps", side_effect=RuntimeError("private")), \
                patch.object(worker, "load_config") as load, contextlib.redirect_stderr(output):
            self.assertEqual(worker.main(["--check", "--config", "/unused"]), 2)
            load.assert_not_called()
        self.assertEqual(output.getvalue(), '{"event":"configuration_rejected"}\n')


if __name__ == "__main__":
    unittest.main()
