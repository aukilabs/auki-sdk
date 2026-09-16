"""Offline tests: no credentials, SDK registration, or live services."""
import asyncio
import hashlib
import importlib.util
import json
import pytest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import AsyncMock

RUN = "11111111-1111-4111-8111-111111111111"
DOMAIN = "22222222-2222-4222-8222-222222222222"


def load(name):
    path = Path(__file__).with_name(name + ".py")
    assert path.exists(), f"missing example module: {name}"
    spec = importlib.util.spec_from_file_location(name, path)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class Data:
    def __init__(self, source=b"Hello, robot!"):
        self.source = source
        self.writes = []
        self.reads = []

    async def read(self, data_id):
        self.reads.append(data_id)
        return self.source

    async def write(self, content, **options):
        self.writes.append((content, options))
        return {"id": "stored-output"}


def task(data, **overrides):
    values = dict(id="task-id", domain_id=DOMAIN,
                  meta={"input_id": "input", "run_id": RUN},
                  data=lambda: data, progress=AsyncMock(), log_event=AsyncMock())
    values.update(overrides)
    return SimpleNamespace(**values)


@pytest.mark.parametrize("overrides", [
    {"domain_id": "wrong-domain"}, {"meta": {"run_id": "wrong-run", "input_id": "input"}},
    {"meta": {"run_id": RUN}},
])
@pytest.mark.parametrize("role", ["compute", "robot"])
def test_handler_rejects_wrong_scope_before_io(overrides, role):
    data = Data()
    with pytest.raises(ValueError):
        asyncio.run(load(role).handle(task(data, **overrides), DOMAIN, RUN))
    assert data.reads == [] and data.writes == []


def test_build_worker_registers_only_run_capability(monkeypatch):
    import sys
    from unittest.mock import Mock
    sdk = SimpleNamespace(AukiComputeCredential=Mock(), AukiRobotCredential=Mock(),
                          AukiDmsTasks=Mock())
    monkeypatch.setitem(sys.modules, "auki_sdk", sdk)
    env = {"DDS_BASE_URL": "http://localhost:1", "DMS_BASE_URL": "http://localhost:2",
           "AUKI_DOMAIN_ID": DOMAIN, "AUKI_RUN_ID": RUN, "AUKI_CLIENT_ID": "stable-test",
           "NODE_REGISTRATION_CREDENTIAL": "placeholder", "NODE_WALLET_KEY": "placeholder",
           "ROBOT_REGISTRATION_CREDENTIAL": "placeholder"}
    common = load("common")
    assert hasattr(common, "build_worker")
    for role in ("compute", "robot"):
        credential, tasks = common.build_worker(role, env)
        capability = f"/examples/compute-robot/{RUN}/{role}/v1"
        assert list(sdk.AukiDmsTasks.call_args.args[1]) == [capability]
        if role == "robot":
            assert sdk.AukiRobotCredential.call_args.kwargs["capabilities"] == [capability]
    with pytest.raises(ValueError):
        common.build_worker("compute", dict(env, AUKI_RUN_ID="not-a-uuid"))


@pytest.mark.parametrize("failure", [None, "start", "close", "timeout"])
def test_lifecycle_polls_until_completed_and_always_closes(failure, capsys):
    common = load("common")
    assert hasattr(common, "run_worker")
    order = []
    class Runtime:
        async def start(self):
            if failure == "start":
                raise RuntimeError("secret token")
        async def run_once(self):
            order.append("poll")
            if failure == "timeout":
                await asyncio.sleep(10)
            return "no_work" if order.count("poll") == 1 else "completed"
        async def close(self):
            order.append("runtime-close")
            if failure == "close":
                raise RuntimeError("secret token")
    class Credential:
        async def close(self):
            order.append("credential-close")
    coroutine = common.run_worker(Credential(), Runtime(), "compute", RUN,
                                  once=True, timeout=0.05, poll_interval=0.001)
    if failure:
        with pytest.raises(Exception):
            asyncio.run(coroutine)
    else:
        asyncio.run(coroutine)
        assert order.count("poll") == 2
        assert json.loads(capsys.readouterr().out.splitlines()[0])["event"] == "ready"
    assert order[-2:] == ["runtime-close", "credential-close"]


@pytest.mark.parametrize("role", ["compute", "robot"])
def test_cli_help_and_redacted_configuration_failure(role):
    import subprocess
    import sys
    script = str(Path(__file__).with_name(role + ".py"))
    help_result = subprocess.run([sys.executable, script, "--help"],
                                 capture_output=True, text=True, env={})
    assert "--once" in help_result.stdout
    result = subprocess.run([sys.executable, script, "--once"],
                            capture_output=True, text=True,
                            env={"NODE_REGISTRATION_CREDENTIAL": "secret-never-print"})
    assert result.returncode == 1
    assert json.loads(result.stderr) == {"event": "error", "message": "worker failed"}
    assert "secret-never-print" not in result.stdout + result.stderr


def test_robot_writes_inspection_report():
    data = Data(b"HELLO, ROBOT!")
    result = asyncio.run(load("robot").handle(task(data), DOMAIN, RUN))
    report = json.loads(data.writes[0][0])
    assert report == {"input_id": "input", "bytes": len(data.source),
                      "sha256": hashlib.sha256(data.source).hexdigest()}
    assert data.writes[0][1]["name"] == f"sdk-{RUN}-robot-task-id"
    assert result["meta"]["sha256"] == report["sha256"]
    assert result["meta"]["bytes"] == report["bytes"]
    assert result["meta"]["data_id"] == result["output_cids"][0]


def test_compute_reads_uppercases_and_returns_verifiable_artifact():
    compute = load("compute")
    data = Data()
    result = asyncio.run(compute.handle(task(data), DOMAIN, RUN))
    assert data.reads == ["input"]
    assert data.writes[0][0] == b"HELLO, ROBOT!"
    assert data.writes[0][1]["name"] == f"sdk-{RUN}-compute-task-id"
    assert result == {"output_cids": ["stored-output"], "meta": {
        "data_id": "stored-output", "run_id": RUN,
        "sha256": hashlib.sha256(b"HELLO, ROBOT!").hexdigest(), "bytes": 13}}
