"""Robot worker: inspect compute output; no hardware actions."""
import hashlib
import json
from common import validate_task


async def handle(task, domain_id, run_id):
    validate_task(task, domain_id, run_id)
    data = task.data()
    await task.progress({"phase": "reading"})
    await task.log_event({"phase": "started", "run_id": run_id})
    source = await data.read(task.meta["input_id"])
    report = {"input_id": task.meta["input_id"], "bytes": len(source),
              "sha256": hashlib.sha256(source).hexdigest()}
    await task.progress({"phase": "writing"})
    stored = await data.write(json.dumps(report, sort_keys=True).encode("utf-8"),
                              name=f"sdk-{run_id}-robot-{task.id}",
                              data_type="example.report.v1")
    await task.log_event({"phase": "finished", "run_id": run_id})
    return {"output_cids": [stored["id"]], "meta": {
        "data_id": stored["id"], "run_id": run_id,
        "sha256": report["sha256"], "bytes": report["bytes"]}}


if __name__ == "__main__":
    from common import main
    raise SystemExit(main("robot"))
