"""Offline contract tests for the opt-in dev operator."""
import importlib.util
from pathlib import Path
import uuid

MODULE = Path(__file__).with_name("compute_robot_operator.py")
RUN = "11111111-1111-4111-8111-111111111111"
DOMAIN = "22222222-2222-4222-8222-222222222222"
INPUT = "33333333-3333-4333-8333-333333333333"


def load():
    assert MODULE.exists(), "operator implementation is missing"
    spec = importlib.util.spec_from_file_location("compute_robot_operator", MODULE)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def test_job_is_dedicated_run_isolated_and_single_attempt():
    op = load()
    body = op.job_payload(DOMAIN, RUN, "compute", INPUT)
    task = body["tasks"][0]
    assert body["domain_id"] == DOMAIN
    assert task["mode"] == "dedicated"
    assert task["max_attempts"] == 1
    assert task["capability"] == f"/examples/compute-robot/{RUN}/compute/v1"
    assert task["meta"] == {"run_id": RUN, "input_id": INPUT}
    assert task["inputs_cids"] == [INPUT]


def test_rejects_scope_overrides_and_noncanonical_ids():
    import pytest
    op = load()

    with pytest.raises(ValueError):
        op.job_payload(DOMAIN, RUN, "robot", INPUT, {"run_id": RUN})
    with pytest.raises(ValueError):
        op.job_payload(DOMAIN, RUN, "other", INPUT)
    with pytest.raises(ValueError):
        op.identifier("{" + RUN + "}")


def test_operator_rejects_non_dev_destinations_before_network():
    import pytest
    op = load()
    with pytest.raises(ValueError):
        op.Operator("https://dms.staging.aukiverse.com/v1", "https://domain-s3.dev.aukiverse.com", DOMAIN, RUN, "fixture")
    with pytest.raises(ValueError):
        op.Operator("https://dms.dev.aukiverse.com/v1", "https://evil.invalid", DOMAIN, RUN, "fixture")


def test_operator_cannot_cancel_an_unowned_job():
    import pytest
    op = load()
    client = op.Operator("https://dms.dev.aukiverse.com/v1", "https://domain-s3.dev.aukiverse.com", DOMAIN, RUN, "fixture")
    with pytest.raises(ValueError):
        client.cancel(INPUT)


def test_multipart_preserves_bytes_and_safe_record_name():
    from email.parser import BytesParser
    from email.policy import default
    op = load()
    payload = b"hello\x00world\r\n"
    body, content_type = op.multipart(f"sdk-{RUN}-input", payload)
    msg = BytesParser(policy=default).parsebytes(f"Content-Type: {content_type}\r\n\r\n".encode() + body)
    part = next(msg.iter_parts())
    assert part.get_payload(decode=True) == payload
    assert part.get_param("name", header="Content-Disposition") == f"sdk-{RUN}-input"


def test_single_task_requires_matching_run_domain_and_capability():
    import pytest
    op = load()
    task = {"id": INPUT, "capability": f"/examples/compute-robot/{RUN}/compute/v1", "meta": {"run_id": RUN}}
    details = {"job": {"domain_id": DOMAIN, "meta": {"run_id": RUN}}, "tasks": [task]}
    assert op.single_task(details, DOMAIN, RUN, "compute") == task
    details["job"]["domain_id"] = INPUT
    with pytest.raises(ValueError):
        op.single_task(details, DOMAIN, RUN, "compute")


def test_submission_waits_for_estimate_before_non_retried_create(monkeypatch):
    op = load()
    client = op.Operator("https://dms.dev.aukiverse.com/v1", "https://domain-s3.dev.aukiverse.com", DOMAIN, RUN, "fixture")
    calls = []
    def request(method, url, body=None):
        calls.append(url)
        if url.endswith("/estimate"):
            if len(calls) == 1:
                raise op.HttpFailure(400)
            return {}
        if url.endswith("/jobs"):
            assert calls[-2].endswith("/estimate"), "job posted before availability established"
            return {"job_id": INPUT}
        return {"job": {"domain_id": DOMAIN, "meta": {"run_id": RUN}},
                "tasks": [{"id": INPUT, "capability": f"/examples/compute-robot/{RUN}/compute/v1", "meta": {"run_id": RUN}}]}
    client.request = request
    monkeypatch.setattr(op.time, "sleep", lambda _: None)
    assert client.submit("compute", INPUT) == INPUT
    assert sum(url.endswith("/jobs") for url in calls) == 1


def test_cleanup_refuses_unrelated_record():
    import pytest
    op = load()
    client = op.Operator("https://dms.dev.aukiverse.com/v1", "https://domain-s3.dev.aukiverse.com", DOMAIN, RUN, "fixture")
    client.names.add(f"sdk-{RUN}-input")
    calls = []
    def request(method, url, *args):
        calls.append(method)
        return {"data": [{"id": INPUT, "name": "other-run", "domain_id": DOMAIN}]}
    client.request = request
    with pytest.raises(ValueError):
        client.cleanup_data()
    assert calls == ["GET"]


def test_cli_dry_run_needs_no_credentials_or_network():
    import os
    import subprocess
    import sys
    env = {"PATH": os.environ["PATH"], "AUKI_DOMAIN_ID": DOMAIN, "AUKI_RUN_ID": RUN}
    script = Path(__file__).with_name("run-compute-robot.py")
    result = subprocess.run([sys.executable, str(script)], env=env, capture_output=True, text=True, timeout=5)
    assert result.returncode == 0
    assert "Dry run" in result.stdout


def test_operator_sigterm_persists_initial_run_manifest(tmp_path):
    import json
    import os
    import subprocess
    import sys
    directory = str(Path(__file__).parent.resolve())
    report = tmp_path / "report.json"
    source = tmp_path / "input.txt"
    token = tmp_path / "token"
    source.write_text("hello")
    token.write_text("fixture-token")
    code = f'''
import os, runpy, signal, sys
sys.path.insert(0, {directory!r})
import compute_robot_operator
class Interrupted:
    def __init__(self, *args): self.jobs = {{}}; self.names = set()
    def write_input(self, data): os.kill(os.getpid(), signal.SIGTERM)
compute_robot_operator.Operator = Interrupted
sys.argv = ['run-compute-robot.py', '--live', '--input-file', {str(source)!r}, '--report', {str(report)!r}]
runpy.run_path({str(Path(directory) / 'run-compute-robot.py')!r}, run_name='__main__')
'''
    env = dict(os.environ, AUKI_DOMAIN_ID=DOMAIN, AUKI_RUN_ID=RUN,
               AUKI_DOMAIN_TOKEN_FILE=str(token), DMS_BASE_URL="https://dms.dev.aukiverse.com/v1",
               DOMAIN_SERVER_URL="https://domain-s3.dev.aukiverse.com")
    result = subprocess.run([sys.executable, "-c", code], env=env, capture_output=True, text=True, timeout=10)
    assert result.returncode == 1, "SIGTERM skipped controlled cleanup"
    manifest = json.loads(report.read_text())
    assert manifest["run_id"] == RUN and manifest["domain_id"] == DOMAIN
    assert manifest["passed"] is False
    assert "fixture-token" not in report.read_text() + result.stdout + result.stderr
