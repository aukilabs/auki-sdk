import asyncio
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import AsyncMock
from runner import configuration, failure_category, reject_task, serve, close_resources


class RunnerTests(unittest.IsolatedAsyncioTestCase):
    async def test_idle_start_and_cleanup_without_claims(self):
        order = []
        def resource(name):
            async def close():
                order.append(name)
            return SimpleNamespace(close=close)
        credential, tasks, echo = [resource(n) for n in ("credential", "tasks", "echo")]
        tasks.start = AsyncMock()
        peer = SimpleNamespace(wait_stopped=lambda: asyncio.sleep(100))
        tasks.peer = lambda: peer
        config = SimpleNamespace(with_dds_tracker=lambda *_: None)
        sdk = SimpleNamespace(AukiPeerConfig=SimpleNamespace(new=lambda _: config),
                              AukiRobotCredential=lambda **_: credential,
                              AukiDmsTasks=lambda *_args, **_kw: tasks,
                              AukiEcho=SimpleNamespace(mount=AsyncMock(return_value=echo)))
        stop = asyncio.Event()
        stop.set()
        await serve(dict.fromkeys(("DDS_URL", "DMS_URL", "ROBOT_REGISTRATION", "ROBOT_AUDIENCE", "IDENTITY_FILE"), "fixture"), stop, sdk)
        tasks.start.assert_awaited_once()
        self.assertEqual(order, ["echo", "tasks", "credential"])

    async def test_cancellation_keeps_close_order_and_observation(self):
        order = []
        entered, release = asyncio.Event(), asyncio.Event()
        async def echo_close():
            order.append("echo begin")
            entered.set()
            await release.wait()
            order.append("echo end")
        async def tasks_close():
            order.append("tasks")
        async def credential_close():
            order.append("credential")
        operation = asyncio.create_task(close_resources(
            SimpleNamespace(close=echo_close), SimpleNamespace(close=tasks_close),
            SimpleNamespace(close=credential_close)))
        await entered.wait()
        operation.cancel()
        await asyncio.sleep(0)
        operation.cancel()
        await asyncio.sleep(0)
        self.assertFalse(operation.done())
        self.assertEqual(order, ["echo begin"])
        release.set()
        with self.assertRaises(asyncio.CancelledError):
            await operation
        self.assertEqual(order, ["echo begin", "echo end", "tasks", "credential"])

    async def test_task_invocation_is_rejected(self):
        with self.assertRaises(RuntimeError):
            await reject_task(None)

    async def test_explicit_configuration_and_private_identity(self):
        with self.assertRaises(ValueError):
            configuration({})
        with tempfile.TemporaryDirectory() as directory:
            env = {"AUKI_DDS_URL": "http://127.0.0.1:1", "AUKI_DMS_URL": "http://127.0.0.1:1",
                   "AUKI_ROBOT_AUDIENCE": "fixture", "AUKI_ROBOT_REGISTRATION": "synthetic",
                   "AUKI_IDENTITY_FILE": directory + "/identity"}
            self.assertEqual(configuration(env)["ROBOT_AUDIENCE"], "fixture")
            env["AUKI_DDS_URL"] = "http://shared.example.com"
            with self.assertRaises(ValueError):
                configuration(env)

    async def test_start_failure_closes_tasks_before_credential(self):
        order = []
        async def close_tasks():
            order.append("tasks")
            raise RuntimeError("synthetic cleanup failure")
        async def close_credential():
            order.append("credential")
        tasks = SimpleNamespace(start=AsyncMock(side_effect=RuntimeError("synthetic startup failure")), close=close_tasks)
        credential = SimpleNamespace(close=close_credential)
        config = SimpleNamespace(with_dds_tracker=lambda *_: None)
        sdk = SimpleNamespace(AukiPeerConfig=SimpleNamespace(new=lambda _: config),
                              AukiRobotCredential=lambda **_: credential,
                              AukiDmsTasks=lambda *_args, **_kw: tasks)
        with self.assertRaisesRegex(RuntimeError, "robot cleanup failed"):
            await serve(dict.fromkeys(("DDS_URL", "DMS_URL", "ROBOT_REGISTRATION", "ROBOT_AUDIENCE", "IDENTITY_FILE"), "fixture"), asyncio.Event(), sdk)
        self.assertEqual(order, ["tasks", "credential"])


class DiagnosticTests(unittest.TestCase):
    def test_backend_text_never_becomes_diagnostic_output(self):
        error = RuntimeError("raw credential secret and backend body")
        error.kind = "untrusted secret"
        self.assertEqual(failure_category(error), "unclassified")
        error.kind = "authority"
        self.assertEqual(failure_category(error), "authority")
        self.assertEqual(failure_category(RuntimeError("task peer startup failed")), "peer_startup")
