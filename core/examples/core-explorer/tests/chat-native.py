"""Loopback transport fixture using the production inbox/CLI command consumer."""
import asyncio
import importlib.util
import json
import os
from pathlib import Path
import signal
from urllib.parse import urlparse


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


base = Path(__file__).parents[1]
runner = load('idle_runner', base / 'robot' / 'runner.py')
ops = load('chat_operator', base / 'chat-host' / 'inbox.py')


async def main():
    import auki_portable_echo as sdk
    config = runner.configuration(os.environ)
    for name in ('DDS_URL', 'DMS_URL', 'ROBOT_AUDIENCE'):
        if urlparse(config[name]).hostname != '127.0.0.1':
            raise ValueError('fixture must use loopback services')
    spool = ops.Spool(os.environ['AUKI_CHAT_SPOOL'])
    lock = spool.lock('runner.lock')
    spool.fence()
    stop = asyncio.Event()
    failures = []
    original = sdk.AukiEcho

    class Combined:
        @staticmethod
        async def mount(peer):
            echo = await original.mount(peer)
            try:
                host = await sdk.AukiChatHost.mount(peer)
            except BaseException:
                await echo.close()
                raise
            try:
                inbox = ops.Inbox(spool, host)
            except BaseException:
                try:
                    await host.close()
                finally:
                    await echo.close()
                raise
            tasks = [asyncio.create_task(ops.receive_events(inbox)),
                     asyncio.create_task(ops.consume_commands(inbox))]
            closing = False

            def monitor(task):
                if not closing and not task.cancelled():
                    error = task.exception()
                    failures.append(error or RuntimeError('consumer stopped unexpectedly'))
                    stop.set()

            for task in tasks:
                task.add_done_callback(monitor)

            class Owner:
                async def close(self):
                    nonlocal closing
                    closing = True
                    errors = []
                    try:
                        inbox.shutdown()
                    except BaseException as error:
                        errors.append(error)
                    for task in tasks:
                        task.cancel()
                    results = await asyncio.gather(*tasks, return_exceptions=True)
                    errors.extend(result for result in results if isinstance(result, BaseException)
                                  and not isinstance(result, asyncio.CancelledError))
                    for resource in (host, echo):
                        try:
                            await resource.close()
                        except BaseException as error:
                            errors.append(error)
                    if errors:
                        raise RuntimeError('Chat fixture cleanup failed') from errors[0]
            return Owner()

    sdk.AukiEcho = Combined
    for sig in (signal.SIGINT, signal.SIGTERM):
        asyncio.get_running_loop().add_signal_handler(sig, stop.set)
    try:
        await runner.serve(config, stop, sdk=sdk,
                           ready=lambda value: print(json.dumps(value), flush=True))
        if failures:
            raise RuntimeError('Chat fixture consumer failed') from failures[0]
    finally:
        spool.fence()
        lock.close()


if __name__ == '__main__':
    try:
        asyncio.run(main())
    except BaseException:
        print('{"state":"failed","category":"chat_fixture_failed"}', flush=True)
        raise SystemExit(1)
