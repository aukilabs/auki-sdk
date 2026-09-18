"""User peer serving Echo and approved Chat. No task runtime or text execution."""
import argparse
import asyncio
import ctypes
import os
from pathlib import Path
import resource
import signal

# Filename deliberately loaded explicitly to avoid shadowing Python's operator.
import importlib.util
_spec = importlib.util.spec_from_file_location('chat_operator', Path(__file__).with_name('inbox.py'))
_ops = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_ops)


async def serve(args):
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    if ctypes.CDLL(None).prctl(4, 0, 0, 0, 0) != 0:
        raise RuntimeError('hardening failed')
    os.umask(0o077)
    spool = _ops.Spool(args.root)
    lock = spool.lock('runner.lock')
    stop = asyncio.Event()
    loop = asyncio.get_running_loop()
    for sig in (signal.SIGINT, signal.SIGTERM):
        loop.add_signal_handler(sig, stop.set)
    session = peer = echo = host = inbox = None
    tasks = []
    failed = False
    operation_failed = False
    try:
        spool.fence()
        identity = spool.root / 'peer.identity'
        if identity.exists() or identity.is_symlink():
            _ops.check(identity)
        credential = _ops.read(Path(args.credentials).absolute(), 16384)
        if credential.get('environment') != 'dev' or not credential.get('email') or not credential.get('password'):
            raise ValueError('invalid credentials')
        from auki_portable_echo import AukiSession, AukiEcho, AukiChatHost
        try:
            session = await asyncio.wait_for(AukiSession.login_dev(credential['email'], credential['password']), 30)
        finally:
            credential.clear()
        if stop.is_set():
            return
        peer = await asyncio.wait_for(session.start_peer(args.domain, str(identity), discovery_mode='discover_and_advertise'), 40)
        echo = await asyncio.wait_for(AukiEcho.mount(peer), 10)
        host = await asyncio.wait_for(AukiChatHost.mount(peer), 10)
        inbox = _ops.Inbox(spool, host)
        _ops.atomic(spool.root / 'status.json', {'state': 'ready', 'peer_id': peer.peer_id,
                    'domain': args.domain, 'wss_route': peer.routes.wss, 'expires_at': None})

        tasks = [asyncio.create_task(_ops.receive_events(inbox)),
                 asyncio.create_task(_ops.consume_commands(inbox)),
                 asyncio.ensure_future(peer.wait_stopped()), asyncio.create_task(stop.wait())]
        done, _ = await asyncio.wait(tasks, return_when=asyncio.FIRST_COMPLETED)
        for task in done:
            task.result()
        if not stop.is_set():
            raise RuntimeError('host stopped unexpectedly')
    except BaseException:
        operation_failed = True
        raise
    finally:
        # Fence submissions before any awaited cleanup and purge queued plaintext.
        try:
            if inbox:
                inbox.shutdown()
            else:
                spool.fence()
        except BaseException:
            failed = True
        # Close protocol before peer; interrupt next_event before joining waiters.
        for obj, method in ((host, 'close'), (echo, 'close')):
            if obj is not None:
                try:
                    await getattr(obj, method)()
                except BaseException:
                    failed = True
        for task in tasks:
            task.cancel()
        await asyncio.gather(*tasks, return_exceptions=True)
        for obj, method in ((peer, 'shutdown'), (session, 'close')):
            if obj is not None:
                try:
                    await getattr(obj, method)()
                except BaseException:
                    failed = True
        _ops.atomic(spool.root / 'status.json', {'state': 'failed' if failed or operation_failed else 'stopped'})
        for sig in (signal.SIGINT, signal.SIGTERM):
            loop.remove_signal_handler(sig)
        lock.close()
        if failed:
            raise RuntimeError('cleanup failed')


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--root', required=True)
    parser.add_argument('--credentials', required=True)
    parser.add_argument('--domain', required=True)
    arguments = parser.parse_args()
    try:
        _ops.valid_id(arguments.domain)
        asyncio.run(serve(arguments))
    except BaseException:
        print('{"state":"failed","category":"host_operation_failed"}', flush=True)
        raise SystemExit(1)
