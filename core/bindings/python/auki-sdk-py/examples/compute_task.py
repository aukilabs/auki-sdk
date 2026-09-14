"""Run an already-provisioned compute node against explicitly configured services.

Live operation: registers /example/uppercase/v1, claims tasks, reads input data,
and creates result data. Provision/submit only within an approved test scope.
Task meta must contain input_id. The submitter owns result cleanup.
"""
import asyncio
import os
import signal

import auki_sdk

CAPABILITY = "/example/uppercase/v1"


async def uppercase(task):
    data = task.data()
    await task.progress({"phase": "reading"})
    source = await data.read(task.meta["input_id"])
    # This example handles small buffers. Use streaming for large artifacts and
    # a cooperative stop mechanism for CPU-heavy work, threads or hardware.
    result = source.decode("utf-8").upper().encode("utf-8")
    await task.progress({"phase": "writing"})
    stored = await data.write(result, name=f"uppercase-{task.id}", data_type="example.text.v1")
    return {"output_cids": [stored["id"]], "meta": {"data_id": stored["id"]}}


async def main():
    credential = auki_sdk.AukiComputeCredential(
        dds_url=os.environ["DDS_BASE_URL"],
        dms_url=os.environ["DMS_BASE_URL"],
        registration=os.environ["NODE_REGISTRATION_CREDENTIAL"],
        wallet_key=os.environ["NODE_WALLET_KEY"],
        version="1.0.0",
        client_id=os.environ["AUKI_CLIENT_ID"],
        peer_identity_file=os.environ.get("AUKI_PEER_IDENTITY_FILE"),
    )
    tasks = auki_sdk.AukiDmsTasks(credential, {CAPABILITY: uppercase})
    running = asyncio.ensure_future(tasks.run())
    loop = asyncio.get_running_loop()
    for sig in (signal.SIGINT, signal.SIGTERM):
        loop.add_signal_handler(sig, running.cancel)
    try:
        await running
    except asyncio.CancelledError:
        pass
    finally:
        try:
            # Wait for the actual handler's finally blocks, data operations and
            # registration shutdown even when the run awaitable was cancelled.
            await tasks.close()
        finally:
            await credential.close()
            for sig in (signal.SIGINT, signal.SIGTERM):
                loop.remove_signal_handler(sig)


if __name__ == "__main__":
    asyncio.run(main())
