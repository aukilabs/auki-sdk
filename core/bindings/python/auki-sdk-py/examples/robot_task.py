"""Run a provisioned robot; its optional peer remains connected between tasks.

Live operation: registers /example/inspect-input/v1, claims work, reads input_id
and writes a report. The submitter owns result cleanup. No hardware is controlled.
"""
import asyncio
import os
import signal

import auki_sdk

CAPABILITY = "/example/inspect-input/v1"


async def inspect_input(task):
    data = task.data()
    await task.progress({"phase": "reading"})
    source = await data.read(task.meta["input_id"])
    peer = task.peer()
    # This optional operation requires a build with the Info protocol enabled.
    # The remote application remains responsible for authorizing its requests.
    if task.meta.get("remote_peer_id"):
        if peer is None:
            raise ValueError("this task requires configured P2P")
        client = auki_sdk.AukiInfoClient(peer)
        await client.fetch_exact(task.meta["remote_peer_id"], task.meta["remote_route"])
    stored = await data.write(f"Read {len(source)} bytes".encode(),
        name=f"inspection-{task.id}", data_type="example.report.v1")
    return {"output_cids": [stored["id"]]}


async def main():
    robot = auki_sdk.AukiRobotCredential(
        dds_url=os.environ["DDS_BASE_URL"], dms_url=os.environ["DMS_BASE_URL"],
        registration=os.environ["ROBOT_REGISTRATION_CREDENTIAL"],
        audience=os.environ.get("DDS_ROBOT_AUDIENCE"),
        version="1.0.0", client_id=os.environ["AUKI_CLIENT_ID"], capabilities=[CAPABILITY],
        peer_identity_file=os.environ.get("AUKI_PEER_IDENTITY_FILE"),
    )
    tasks = auki_sdk.AukiDmsTasks(robot, {CAPABILITY: inspect_input})
    async def serve():
        await tasks.start()
        peer = tasks.peer()
        if peer is not None:
            print(f"Robot peer ready: {peer.peer_id}")
        # Process-wide endpoints may use this peer while idle. Close them before
        # tasks.close(); task-specific endpoints belong in the handler's finally.
        await tasks.run()
    running = asyncio.ensure_future(serve())
    loop = asyncio.get_running_loop()
    for sig in (signal.SIGINT, signal.SIGTERM):
        loop.add_signal_handler(sig, running.cancel)
    try:
        await running
    except asyncio.CancelledError:
        pass
    finally:
        try:
            await tasks.close()
        finally:
            await robot.close()
            for sig in (signal.SIGINT, signal.SIGTERM):
                loop.remove_signal_handler(sig)


if __name__ == "__main__":
    asyncio.run(main())
