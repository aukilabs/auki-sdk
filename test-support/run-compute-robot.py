#!/usr/bin/env python3
"""Submit the paired example using an operator's dev Domain token.

Workers must already be running. Default is offline dry-run; --live creates jobs
and data. Successful runs remove their named data; failures retain data for audit.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import sys
import signal

from compute_robot_operator import Operator, identifier, job_payload


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--live", action="store_true")
    parser.add_argument("--input-file", type=Path)
    parser.add_argument("--report", type=Path)
    parser.add_argument("--timeout", type=int, default=180)
    args = parser.parse_args()
    if not 1 <= args.timeout <= 600:
        parser.error("timeout must be 1..600 seconds per job")
    domain = identifier(os.environ["AUKI_DOMAIN_ID"])
    run_id = identifier(os.environ["AUKI_RUN_ID"])
    if not args.live:
        print(json.dumps(job_payload(domain, run_id, "compute", "00000000-0000-4000-8000-000000000001"), indent=2))
        print("Dry run: no files containing credentials read and no network requests.")
        return
    if not args.input_file or not args.report:
        parser.error("--live requires --input-file and a NEW --report path")
    # Reserve the audit file before the first external mutation; never overwrite it.
    fd = os.open(args.report, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    report = {"run_id": run_id, "domain_id": domain, "passed": False, "jobs": {},
              "names": [f"sdk-{run_id}-input"],
              "planned_job_labels": [f"sdk-{run_id}-{role}" for role in ("compute", "robot")]}
    operator = None
    signals = (signal.SIGINT, signal.SIGTERM)
    previous_handlers = {sig: signal.getsignal(sig) for sig in signals}
    def interrupt(signum, frame):
        # A second signal must not interrupt known-job cancellation/audit writes.
        for sig in signals:
            signal.signal(sig, signal.SIG_IGN)
        raise KeyboardInterrupt
    for sig in signals:
        signal.signal(sig, interrupt)
    with os.fdopen(fd, "w") as output:
        def persist():
            output.seek(0)
            json.dump(report, output, indent=2)
            output.truncate()
            output.flush()
            os.fsync(output.fileno())
        try:
            # Include reconciliation identities even if the first POST response is lost.
            persist()
            with args.input_file.open("rb") as stream:
                source = stream.read(1024 * 1024 + 1)
            expected = source.decode("utf-8").upper().encode("utf-8")
            if not source or max(len(source), len(expected)) > 1024 * 1024:
                raise ValueError("input/output must be nonempty and at most 1 MiB")
            token = Path(os.environ["AUKI_DOMAIN_TOKEN_FILE"]).read_text().strip()
            operator = Operator(os.environ["DMS_BASE_URL"], os.environ["DOMAIN_SERVER_URL"], domain, run_id, token)
            input_id = operator.write_input(source)
            for role in ("compute", "robot"):
                report.update(pending_role=role, names=sorted(operator.names))
                persist()
                extra = {}
                if role == "compute" and os.environ.get("AUKI_REMOTE_PEER_ID"):
                    extra = {"remote_peer_id": os.environ["AUKI_REMOTE_PEER_ID"],
                             "remote_route": os.environ["AUKI_REMOTE_ROUTE"]}
                job_id = operator.submit(role, input_id, extra)
                report["jobs"] = operator.jobs
                persist()
                details = operator.wait(job_id, args.timeout)
                if details["tasks"][0]["status"] != "completed":
                    raise RuntimeError("example task did not complete")
                receipts = details["receipts"]
                if len(receipts) != 1:
                    raise ValueError("expected exactly one completion receipt")
                expected_node = os.environ[f"AUKI_{role.upper()}_ID"]
                if receipts[0]["node_id"] != identifier(expected_node):
                    raise ValueError("unexpected executor")
                data_id = identifier(receipts[0]["meta"]["data_id"])
                content = operator.read(data_id)
                if role == "compute":
                    if content != expected:
                        raise ValueError("compute output differs from expected bytes")
                else:
                    inspection = json.loads(content)
                    if inspection["input_id"] != input_id or inspection["bytes"] != len(expected) or inspection["sha256"] != hashlib.sha256(expected).hexdigest():
                        raise ValueError("robot report does not match independently verified input")
                report[role] = {"job_id": job_id, "task_id": details["tasks"][0]["id"],
                                "node_id": receipts[0]["node_id"], "output_id": data_id,
                                "sha256": hashlib.sha256(content).hexdigest()}
                input_id = data_id
            operator.cleanup_data()
            report.update(passed=True, data_cleanup=True)
            print(json.dumps({"passed": True, "run_id": run_id, "report": str(args.report)}))
        except BaseException as exc:
            report["error_type"] = type(exc).__name__
            if operator is not None:
                report["cancel_errors"] = []
                for job_id in operator.jobs:
                    try:
                        operator.cancel(job_id)
                    except Exception:
                        report["cancel_errors"].append(job_id)
            print("Run failed; details suppressed. Stop workers and reconcile only report-listed jobs/data; data retained.", file=sys.stderr)
            raise SystemExit(1) from None
        finally:
            if operator is not None:
                report.update(jobs=operator.jobs, names=sorted(operator.names))
            try:
                persist()
            finally:
                for sig, handler in previous_handlers.items():
                    signal.signal(sig, handler)


if __name__ == "__main__":
    main()
