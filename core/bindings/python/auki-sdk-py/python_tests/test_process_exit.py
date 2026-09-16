"""Normal interpreter shutdown is part of the Python lifecycle contract."""
import os
import subprocess
import sys

import pytest


GUARD = """
import sys
if sys.platform != "win32":
    import resource
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
if sys.platform.startswith("linux"):
    import ctypes
    if ctypes.CDLL(None).prctl(4, 0, 0, 0, 0) != 0:
        raise RuntimeError("could not disable core dumping")
"""
ROBOT = """
import asyncio
import auki_sdk
async def main():
    robot = auki_sdk.AukiRobotCredential(
        dds_url="http://127.0.0.1:1", dms_url="http://127.0.0.1:1",
        registration="synthetic-unregistered-robot", version="regression",
        client_id="local-process-exit-check", audience="local-fixture",
        capabilities=["/example/noop/v1"],
    )
    await robot.close()
    print("closed", flush=True)
asyncio.run(main())
print("returned", flush=True)
"""


CANCELLED_CLOSE = ROBOT.replace(
    "    await robot.close()",
    """    closing = robot.close()
    closing.cancel()
    try:
        await closing
    except asyncio.CancelledError:
        pass
    await asyncio.gather(robot.close(), robot.close())""",
)


@pytest.mark.parametrize("case,source", [
    ("python_only", "import asyncio\nasync def main(): pass\nasyncio.run(main())\nprint('returned', flush=True)"),
    ("sdk_import_only", "import auki_sdk\nprint('returned', flush=True)"),
    ("robot_construct_close", ROBOT),
    ("cancelled_repeated_close", CANCELLED_CLOSE),
], ids=["python_only", "sdk_import_only", "robot_construct_close", "cancelled_repeated_close"])
def test_normal_process_exit(case, source):
    # A timeout, signal, or nonzero status is a failure even after both markers.
    # Set this to 100 for the release gate; keep the default local run focused.
    attempts = int(os.environ.get("AUKI_PROCESS_EXIT_ATTEMPTS", "20"))
    assert attempts > 0
    for iteration in range(1, attempts + 1):
        result = subprocess.run(
            [sys.executable, "-c", GUARD + source],
            capture_output=True, text=True, timeout=10,
            env={**os.environ, "PYTHONNOUSERSITE": "1"},
        )
        assert result.returncode == 0, (
            f"{case} iteration {iteration}/{attempts}: exit {result.returncode}\n"
            f"stdout: {result.stdout}\nstderr: {result.stderr}"
        )
        assert "returned" in result.stdout.splitlines()
        if case in ("robot_construct_close", "cancelled_repeated_close"):
            assert "closed" in result.stdout.splitlines()


COMPLETION_SETUP = """
import asyncio
import threading
import weakref
import auki_sdk

def robot():
    return auki_sdk.AukiRobotCredential(
        dds_url="http://127.0.0.1:1", dms_url="http://127.0.0.1:1",
        registration="synthetic-unregistered-robot", version="regression",
        client_id="local-completion-check", audience="local-fixture",
        capabilities=["/example/noop/v1"],
    )
"""


def run_completion_child(source):
    result = subprocess.run(
        [sys.executable, "-c", GUARD + COMPLETION_SETUP + source],
        capture_output=True, text=True, timeout=15,
        env={**os.environ, "PYTHONNOUSERSITE": "1",
             "PYTHONPATH": os.path.dirname(os.path.abspath(__file__))},
    )
    assert result.returncode == 0, (
        f"exit {result.returncode}\nstdout: {result.stdout}\nstderr: {result.stderr}"
    )
    assert "returned" in result.stdout.splitlines()


@pytest.mark.parametrize("reject_at", [1, 2])
def test_callback_registration_failure_never_launches_bridge(reject_at):
    run_completion_child("reject_at = " + str(reject_at) + "\n" + """
async def main():
    loop = asyncio.get_running_loop()
    create, publish = loop.create_future, loop.call_soon_threadsafe
    published, release, finished = (threading.Event() for _ in range(3))
    references, callbacks = [], []
    class RejectedFuture(asyncio.Future):
        def add_done_callback(self, callback, **kwargs):
            callbacks.append(type(callback).__name__)
            if len(callbacks) == reject_at:
                raise RuntimeError("injected callback registration failure")
            return super().add_done_callback(callback, **kwargs)
    def create_rejected():
        future = RejectedFuture(loop=loop)
        references.append(weakref.ref(future))
        return future
    def delayed_publish(*args, **kwargs):
        published.set()
        assert release.wait(5), "publisher release timed out"
        result = publish(*args, **kwargs)
        finished.set()
        return result
    def attempt_close(credential):
        try:
            credential.close()
        except RuntimeError as error:
            assert str(error) == "injected callback registration failure"
        else:
            raise AssertionError("registration error was swallowed")
    credential = robot()
    loop.create_future = create_rejected
    loop.call_soon_threadsafe = delayed_publish
    try:
        attempt_close(credential)
        # A bridge owns strong references even before its first poll. A failed
        # conversion must drop those references synchronously, without a join.
        retained = references[0]() is not None
        if retained:
            assert published.wait(5), "retained native bridge did not publish"
    finally:
        release.set()
        loop.create_future = create
        loop.call_soon_threadsafe = publish
    if retained:
        assert finished.wait(5), "native publisher did not finish"
    await credential.close()
    await robot().close()
    assert callbacks == ["PyDoneCallback", "DrainCompletion"][:reject_at]
    assert not retained, "registration error launched native bridge and published a result"
    assert not published.is_set(), "failed registration published native completion"
asyncio.run(main())
print("returned", flush=True)
""")


def test_delayed_native_publisher_finishes_before_awaiter_resumes():
    run_completion_child("""
async def main():
    loop = asyncio.get_running_loop()
    create, publish = loop.create_future, loop.call_soon_threadsafe
    entered, release, finished = (threading.Event() for _ in range(3))
    order, errors = [], []
    class ObservedFuture(asyncio.Future):
        def add_done_callback(self, callback, **kwargs):
            if type(callback).__name__ == "DrainCompletion":
                drain = callback
                def callback(future):
                    order.append("drain entered")
                    entered.set()
                    drain(future)
                    assert finished.is_set()
                    order.append("drain returned")
            return super().add_done_callback(callback, **kwargs)
    def delayed_publish(*args, **kwargs):
        result = publish(*args, **kwargs)
        assert release.wait(5), "publisher release timed out"
        finished.set()
        return result
    def release_publisher():
        try:
            assert entered.wait(5), "drain callback never ran"
            assert not finished.is_set(), "publisher was not delayed"
        except BaseException as error:
            errors.append(error)
        finally:
            release.set()
    controller = threading.Thread(target=release_publisher)
    controller.start()
    loop.create_future = lambda: ObservedFuture(loop=loop)
    loop.call_soon_threadsafe = delayed_publish
    try:
        await robot().close()
        order.append("awaiter resumed")
        assert finished.is_set()
        assert order == ["drain entered", "drain returned", "awaiter resumed"]
    finally:
        release.set()
        loop.create_future = create
        loop.call_soon_threadsafe = publish
        controller.join(5)
    assert not controller.is_alive()
    assert not errors, errors
    await robot().close()
asyncio.run(main())
print("returned", flush=True)
""")


@pytest.mark.parametrize("reenter_at", ["create", "register"])
def test_reentrant_completion_capture_keeps_both_bridges(reenter_at):
    run_completion_child("reenter_at = " + repr(reenter_at) + "\n" + """
async def main():
    loop = asyncio.get_running_loop()
    create = loop.create_future
    nested, callbacks = [], []
    reentered = False
    def reenter():
        nonlocal reentered
        if not reentered:
            reentered = True
            nested.append(robot().close())
    class ObservedFuture(asyncio.Future):
        def add_done_callback(self, callback, **kwargs):
            name = type(callback).__name__
            callbacks.append(name)
            if reenter_at == "register" and name == "DrainCompletion":
                reenter()
            return super().add_done_callback(callback, **kwargs)
    def create_observed():
        if reenter_at == "create":
            reenter()
        return ObservedFuture(loop=loop)
    loop.create_future = create_observed
    try:
        outer = robot().close()
    finally:
        loop.create_future = create
    assert len(nested) == 1
    await asyncio.gather(outer, nested[0])
    assert callbacks.count("PyDoneCallback") == 2
    assert callbacks.count("DrainCompletion") == 2
    await robot().close()
asyncio.run(main())
print("returned", flush=True)
""")
