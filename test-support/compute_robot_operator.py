"""Bounded operator operations for the compute/robot example (dev only)."""
import uuid
import json
import re
import time
import urllib.error
import urllib.parse
import urllib.request
from typing import Any


class HttpFailure(RuntimeError):
    def __init__(self, status):
        self.status = status
        super().__init__(f"HTTP {status}; response body suppressed")


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


def multipart(name, payload):
    if not re.fullmatch(r"[a-zA-Z0-9._-]{1,128}", name):
        raise ValueError("unsafe data name")
    boundary = "sdk-" + str(uuid.uuid4())
    if boundary.encode() in payload or len(payload) > 1024 * 1024:
        raise ValueError("invalid or oversized demo payload")
    headers = (f"--{boundary}\r\nContent-Type: application/octet-stream\r\n"
               f'Content-Disposition: form-data; name="{name}"; data-type="example.text.v1"\r\n\r\n')
    return (headers.encode() + payload + f"\r\n--{boundary}--\r\n".encode(),
            f"multipart/form-data; boundary={boundary}")


def single_task(details, domain, run_id, role):
    job = details["job"]
    if job["domain_id"] != domain or job.get("meta", {}).get("run_id") != run_id:
        raise ValueError("job escaped the approved run/Domain")
    tasks = details["tasks"]
    if len(tasks) != 1:
        raise ValueError("expected exactly one task")
    task = tasks[0]
    if (task["capability"] != f"/examples/compute-robot/{run_id}/{role}/v1"
            or task.get("meta", {}).get("run_id") != run_id):
        raise ValueError("task escaped run/capability")
    return task


class Operator:
    """Use a human-authorized Domain token, never a worker credential.

    This helper intentionally accepts only official dev DMS and data endpoints.
    No mutation is retried: an ambiguous result needs operator reconciliation.
    """
    def __init__(self, dms, server, domain, run_id, token):
        if dms != "https://dms.dev.aukiverse.com/v1":
            raise ValueError("operator requires the exact dev DMS /v1 endpoint")
        if server not in {"https://domain-s3.dev.aukiverse.com", "https://domain.dev.aukiverse.com"}:
            raise ValueError("operator requires an approved dev Domain Server")
        self.dms, self.server = dms, server
        self.domain, self.run_id = identifier(domain), identifier(run_id)
        self._token = token
        self.jobs = {}
        self.names = set()
        self.http = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())

    def request(self, method, url, body=None, content_type=None, raw=False) -> Any:
        if not (url.startswith(self.dms + "/") or url.startswith(self.server + "/api/v1/domains/" + self.domain + "/data")):
            raise ValueError("request outside approved origins/Domain")
        headers = {"Authorization": "Bearer " + self._token, "Accept": "application/json"}
        if body is not None:
            if content_type is None:
                body = json.dumps(body).encode()
                content_type = "application/json"
            headers["Content-Type"] = content_type
        req = urllib.request.Request(url, data=body, method=method, headers=headers)
        try:
            with self.http.open(req, timeout=20) as response:
                data = response.read(2 * 1024 * 1024 + 1)
                if len(data) > 2 * 1024 * 1024:
                    raise ValueError("response exceeds demo bound")
        except urllib.error.HTTPError as exc:
            raise HttpFailure(exc.code) from None
        except (OSError, urllib.error.URLError):
            raise RuntimeError("network request failed; details suppressed") from None
        return data if raw else (json.loads(data) if data else None)

    @property
    def data_url(self):
        return f"{self.server}/api/v1/domains/{self.domain}/data"

    def write_input(self, payload):
        name = f"sdk-{self.run_id}-input"
        self.names.add(name)  # Track before POST, including an ambiguous response.
        body, kind = multipart(name, payload)
        result = self.request("POST", self.data_url, body, kind)["data"]
        if len(result) != 1:
            raise ValueError("expected one stored input")
        saved = result[0]
        if self.read(saved["id"]) != payload:
            raise ValueError("input readback mismatch")
        return saved["id"]

    def read(self, data_id):
        return self.request("GET", f"{self.data_url}/{identifier(data_id)}?raw=true", raw=True)

    def submit(self, role, input_id, extra_meta=None):
        body = job_payload(self.domain, self.run_id, role, input_id, extra_meta)
        # DDS startup is not DMS availability. Workers must poll at least once.
        # Estimation is read-only; never retry a job-creation POST.
        for attempt in range(20):
            try:
                self.request("POST", self.dms + "/jobs/estimate", body)
                break
            except HttpFailure as exc:
                if exc.status != 400 or attempt == 19:
                    raise
                time.sleep(1)
        result = self.request("POST", self.dms + "/jobs", body)
        job_id = identifier(result["job_id"])
        self.jobs[job_id] = role
        self.job(job_id)  # Read back exact target before reporting success.
        return job_id

    def job(self, job_id):
        if job_id not in self.jobs:
            raise ValueError("job is not owned by this operator run")
        details = self.request("GET", self.dms + "/jobs/" + identifier(job_id))
        task = single_task(details, self.domain, self.run_id, self.jobs[job_id])
        self.names.add(f"sdk-{self.run_id}-{self.jobs[job_id]}-{identifier(task['id'])}")
        return details

    def wait(self, job_id, timeout=180):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            details = self.job(job_id)
            state = details["tasks"][0]["status"]
            if state in {"completed", "failed", "canceled"}:
                return details
            time.sleep(1)
        raise TimeoutError("job did not reach a terminal state")

    def cancel(self, job_id):
        if job_id not in self.jobs:
            raise ValueError("refusing to cancel an unowned job")
        details = self.job(job_id)
        if details["tasks"][0]["status"] not in {"completed", "failed", "canceled"}:
            self.request("POST", self.dms + "/jobs/" + identifier(job_id) + "/cancel", {})
        return self.job(job_id)

    def cleanup_data(self):
        """Call only after workers stopped; reconcile exact run-owned names."""
        for name in sorted(self.names):
            url = self.data_url + "?" + urllib.parse.urlencode({"name": name})
            records = self.request("GET", url)["data"]
            for record in records:
                if record.get("name") != name or record.get("domain_id") != self.domain:
                    raise ValueError("data cleanup scope mismatch")
                self.request("DELETE", self.data_url + "/" + identifier(record["id"]))
            if self.request("GET", url)["data"]:
                raise RuntimeError("run-owned data remains after cleanup")


def identifier(value):
    parsed = str(uuid.UUID(value))
    if parsed != value:
        raise ValueError("identifier must be a canonical UUID")
    return value


def job_payload(domain_id, run_id, role, input_id, extra_meta=None):
    for value in (domain_id, run_id, input_id):
        identifier(value)
    if role not in ("compute", "robot"):
        raise ValueError("unknown worker role")
    meta = dict(extra_meta or {})
    if {"run_id", "input_id"} & meta.keys():
        raise ValueError("cannot override run or input identity")
    meta.update(run_id=run_id, input_id=input_id)
    return {
        "label": f"sdk-{run_id}-{role}", "domain_id": domain_id,
        "priority": 0, "meta": {"run_id": run_id}, "edges": [],
        "tasks": [{"label": role, "stage": role,
                   "capability": f"/examples/compute-robot/{run_id}/{role}/v1",
                   "mode": "dedicated", "max_attempts": 1, "priority": 0,
                   "capability_filters": {}, "inputs_cids": [input_id],
                   "outputs_prefix": None, "meta": meta}],
    }
