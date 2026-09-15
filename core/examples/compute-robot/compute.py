"""Compute worker: read UTF-8 input and write its uppercase form."""
import hashlib
from common import validate_task


async def handle(task, domain_id, run_id, expected_peer_id=None):
    validate_task(task, domain_id, run_id)
    if task.meta.get("remote_peer_id"):
        if task.meta["remote_peer_id"] != expected_peer_id:
            raise ValueError("P2P target is not the configured partner")
        from peer_info import fetch_robot
        await fetch_robot(task, run_id, expected_peer_id)
    data = task.data()
    await task.progress({"phase": "reading"})
    await task.log_event({"phase": "started", "run_id": run_id})
    source = await data.read(task.meta["input_id"])
    content = source.decode("utf-8").upper().encode("utf-8")
    await task.progress({"phase": "writing"})
    stored = await data.write(content, name=f"sdk-{run_id}-compute-{task.id}",
                              data_type="example.text.v1")
    await task.log_event({"phase": "finished", "run_id": run_id})
    return {"output_cids": [stored["id"]], "meta": {
        "data_id": stored["id"], "run_id": run_id,
        "sha256": hashlib.sha256(content).hexdigest(), "bytes": len(content)}}


if __name__ == "__main__":
    from common import main
    raise SystemExit(main("compute"))
