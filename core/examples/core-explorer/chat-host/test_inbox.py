import asyncio
import importlib.util
import os
import json
import subprocess
import sys
import threading
from types import SimpleNamespace
from unittest.mock import patch
from pathlib import Path
import tempfile
import time
import unittest
import uuid
spec = importlib.util.spec_from_file_location('chat_inbox', Path(__file__).with_name('inbox.py'))
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)

class Host:
    def __init__(self):
        self.calls = []
    async def approve(self, sid, peer):
        self.calls.append(('approve', sid, peer))
    async def send(self, sid, mid, text):
        self.calls.append(('send', sid, mid, text))

class InboxTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.spool = mod.Spool(self.temp.name)
        self.host = Host()
        self.inbox = mod.Inbox(self.spool, self.host)
        self.sid = str(uuid.uuid4())
        self.inbox.event({'type': 'pending', 'session_id': self.sid, 'peer_id': 'peer'})
    def tearDown(self):
        self.temp.cleanup()
    def event(self, kind, **values):
        self.inbox.event({'type': kind, 'session_id': self.sid, 'peer_id': 'peer', **values})
    def command(self, action='approve', **values):
        return {'run': self.inbox.run, 'action': action, 'session_id': self.sid, 'peer_id': 'peer', 'created': time.time(), 'id': str(uuid.uuid4()), **values}
    async def test_exact_pair_and_preapproval_denial(self):
        with self.assertRaises(ValueError):
            self.event('message', id=str(uuid.uuid4()), text='must not record')
        self.assertEqual(self.inbox.sessions[self.sid]['messages'], [])
        with self.assertRaises(ValueError):
            await self.inbox.command(self.command(peer_id='wrong'))
        with self.assertRaises(ValueError):
            await self.inbox.command(self.command('reply', text='bad'))
        await self.inbox.command(self.command())
        self.assertEqual(self.host.calls, [('approve', self.sid, 'peer')])
    async def test_receipt_and_disconnect(self):
        self.event('paired')
        cmd = self.command('reply', text='custom response')
        await self.inbox.command(cmd)
        self.assertEqual(self.inbox.sessions[self.sid]['messages'][0]['status'], 'sent')
        self.event('ack', id=cmd['id'])
        self.assertEqual(self.inbox.sessions[self.sid]['messages'][0]['status'], 'received by peer')
        self.event('closed')
        with self.assertRaises(ValueError):
            await self.inbox.command(self.command('reply', text='stale'))
    async def test_bounds_replay_and_expiry(self):
        self.event('paired')
        for body in ['', 'é' * 1025]:
            with self.assertRaises(ValueError):
                await self.inbox.command(self.command('reply', text=body))
        with self.assertRaises(ValueError):
            self.event('ack', id=str(uuid.uuid4()))
        with self.assertRaises(ValueError):
            await self.inbox.command(self.command(run='old-process'))
        self.inbox.sessions[self.sid]['expires'] = time.time() - 1
        with self.assertRaises(ValueError):
            await self.inbox.command(self.command())
        self.inbox.save()
        self.assertEqual(self.inbox.sessions, {})
    async def test_private_atomic_queue_and_restart(self):
        with self.assertRaises(ValueError):
            self.spool.submit('approve', self.sid, 'wrong')
        cid = self.spool.submit('approve', self.sid, 'peer')
        path = self.spool.commands / (cid + '.json')
        self.assertEqual(path.stat().st_mode & 0o777, 0o600)
        mod.Inbox(self.spool, self.host)
        self.assertFalse(path.exists())
        link = Path(self.temp.name) / 'link'
        link.symlink_to('/etc/passwd')
        with self.assertRaises((ValueError, OSError)):
            mod.read(link)

class RepairTests(InboxTests):
    async def test_sequential_reconnects_and_terminal_events(self):
        for _ in range(10):
            self.event('closed')
            self.event('closed')
            self.sid = str(uuid.uuid4())
            self.event('pending')
            self.assertLessEqual(len(self.inbox.sessions), 4)
        self.inbox.sessions[self.sid]['expires'] = time.time() - 1
        self.inbox.save()
        self.event('closed')
        self.event('closed')
        self.assertNotIn(self.sid, self.inbox.sessions)
        self.assertFalse(self.inbox.accept({'type': 'message', 'session_id': self.sid,
                                           'peer_id': 'peer', 'id': str(uuid.uuid4()), 'text': 'late'}))
        self.assertNotIn(self.sid, self.inbox.sessions)

    async def test_session_failure_does_not_stop_other_session_consumer(self):
        other = str(uuid.uuid4())
        events = [
            {'type': 'message', 'session_id': self.sid, 'peer_id': 'peer', 'id': str(uuid.uuid4()), 'text': 'not approved'},
            {'type': 'pending', 'session_id': other, 'peer_id': 'other'},
            {'type': 'paired', 'session_id': other, 'peer_id': 'other'},
            {'type': 'message', 'session_id': other, 'peer_id': 'other', 'id': str(uuid.uuid4()), 'text': 'valid'},
        ]
        queue = asyncio.Queue()
        for event in events:
            queue.put_nowait(json.dumps(event))
        self.host.next_event = queue.get
        task = asyncio.create_task(mod.receive_events(self.inbox))
        await asyncio.sleep(0)
        self.assertFalse(task.done())
        self.assertEqual(self.inbox.sessions[other]['messages'][0]['text'], 'valid')
        self.assertEqual(self.inbox.sessions[self.sid]['state'], 'closed')
        task.cancel()
        await asyncio.gather(task, return_exceptions=True)

    async def test_runner_receiver_survives_more_than_four_reconnects(self):
        queue = asyncio.Queue()
        queue.put_nowait(json.dumps({'type': 'closed', 'session_id': self.sid, 'peer_id': 'peer'}))
        for _ in range(8):
            sid = str(uuid.uuid4())
            for kind in ('pending', 'closed', 'closed'):
                queue.put_nowait(json.dumps({'type': kind, 'session_id': sid, 'peer_id': 'peer'}))
        last = str(uuid.uuid4())
        queue.put_nowait(json.dumps({'type': 'pending', 'session_id': last, 'peer_id': 'peer'}))
        self.host.next_event = queue.get
        receiver = asyncio.create_task(mod.receive_events(self.inbox))
        try:
            await asyncio.sleep(0)
            self.assertFalse(receiver.done())
            self.assertEqual(self.inbox.sessions[last]['state'], 'pending')
            self.assertLessEqual(len(self.inbox.sessions), 4)
        finally:
            receiver.cancel()
            await asyncio.gather(receiver, return_exceptions=True)

    async def test_publish_window_never_claims_temporary_file(self):
        command = self.command()
        final = self.spool.commands / (command['id'] + '.json')
        replace = os.replace
        observed = []
        def publish(temporary, destination):
            observed.append(temporary.exists())
            self.assertIsNone(self.spool.claim())
            self.assertTrue(temporary.exists())
            replace(temporary, destination)
        with patch.object(mod.os, 'replace', publish):
            mod.atomic(final, command)
        self.assertEqual(observed, [True])
        self.assertEqual(self.spool.claim(), command)
        self.assertIsNone(self.spool.claim())
        mod.atomic(self.spool.commands / 'not-a-uuid.json', command)
        self.assertIsNone(self.spool.claim())

    async def test_shutdown_and_abandoned_plaintext_retention(self):
        self.event('paired')
        self.spool.submit('reply', self.sid, 'peer', 'private queued reply')
        abandoned = self.spool.commands / ('.tmp-' + str(uuid.uuid4()))
        abandoned.write_text('partial private reply')
        abandoned.chmod(0o600)
        old = time.time() - mod.COMMAND_TTL - 1
        os.utime(abandoned, (old, old))
        with self.spool.lock('queue.lock'):
            self.spool.purge()
        self.assertFalse(abandoned.exists())
        self.inbox.shutdown()
        self.assertEqual(list(self.spool.commands.iterdir()), [])
        self.assertEqual(self.spool.snapshot()['sessions'], {})
        self.assertFalse(self.spool.snapshot()['active'])
        with self.assertRaises(ValueError):
            self.spool.submit('reply', self.sid, 'peer', 'after shutdown')
        with self.assertRaises(ValueError):
            await self.inbox.command(self.command('reply', text='late claimed command'))
        self.inbox.save()
        self.assertFalse(self.spool.snapshot()['active'])

    async def test_shutdown_serializes_with_in_progress_publication(self):
        self.event('paired')
        publishing, release, fenced = threading.Event(), threading.Event(), threading.Event()
        errors = []
        replace = os.replace
        def pause_publish(source, destination):
            if destination.parent == self.spool.commands:
                publishing.set()
                if not release.wait(2):
                    raise RuntimeError('test publish barrier timed out')
            replace(source, destination)
        def submit():
            try:
                self.spool.submit('reply', self.sid, 'peer', 'queued during stop')
            except BaseException as error:
                errors.append(error)
        def shutdown():
            try:
                self.inbox.shutdown()
            except BaseException as error:
                errors.append(error)
            finally:
                fenced.set()
        with patch.object(mod.os, 'replace', pause_publish):
            sender = threading.Thread(target=submit)
            sender.start()
            self.assertTrue(publishing.wait(2))
            stopper = threading.Thread(target=shutdown)
            stopper.start()
            try:
                self.assertFalse(fenced.wait(0.05), 'shutdown must wait for atomic publication')
            finally:
                release.set()
                sender.join(2)
                stopper.join(2)
        self.assertTrue(fenced.is_set())
        self.assertEqual(errors, [])
        self.assertFalse(self.spool.snapshot()['active'])
        self.assertEqual(list(self.spool.commands.iterdir()), [])

    async def test_runner_startup_failure_fences_old_authority(self):
        spec = importlib.util.spec_from_file_location('chat_runner_test', Path(__file__).with_name('runner.py'))
        runner = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(runner)
        self.event('paired')
        self.spool.submit('reply', self.sid, 'peer', 'old queued text')
        credentials = self.spool.root / 'mock-credentials.json'
        mod.atomic(credentials, {'environment': 'invalid-fixture'})
        args = SimpleNamespace(root=self.temp.name, credentials=str(credentials), domain=str(uuid.uuid4()))
        with patch.object(runner.resource, 'setrlimit'), patch.object(runner.ctypes, 'CDLL') as libc, patch.object(runner.os, 'umask'):
            libc.return_value.prctl.return_value = 0
            with self.assertRaises(ValueError):
                await runner.serve(args)
        self.assertEqual(list(self.spool.commands.iterdir()), [])
        self.assertFalse(self.spool.snapshot()['active'])
        self.assertEqual(self.spool.snapshot()['sessions'], {})
        self.assertEqual(mod.read(self.spool.root / 'status.json')['state'], 'failed')

    async def test_maximum_escaped_snapshot_budget(self):
        # Build all valid events in memory, then write only ONE final 6+ MB snapshot.
        save = self.inbox.save
        self.inbox.save = lambda: None
        for index in range(4):
            if index:
                self.sid = str(uuid.uuid4())
                self.event('pending')
            self.event('paired')
            for _ in range(128):
                self.event('message', id=str(uuid.uuid4()), text='\x00' * 2048)
        self.inbox.save = save
        save()
        snapshot = self.spool.snapshot()
        self.assertEqual(len(snapshot['sessions']), 4)
        self.assertTrue(all(len(value['messages']) == 128 for value in snapshot['sessions'].values()))
        self.assertLess((self.spool.root / 'inbox.json').stat().st_size, mod.LIMIT)
        with self.assertRaises(ValueError):
            self.event('message', id=str(uuid.uuid4()), text='over capacity')

    async def test_real_cli_and_production_command_consumer(self):
        def cli(action, body=None, peer='peer'):
            result = subprocess.run([sys.executable, str(Path(mod.__file__)), '--root', self.temp.name,
                                     action, '--session', self.sid, '--peer', peer],
                                    input=body, text=True, capture_output=True, timeout=5)
            self.assertEqual(result.returncode, 0, result.stderr)
            return json.loads(result.stdout)
        cli('approve')
        consumer = asyncio.create_task(mod.consume_commands(self.inbox))
        try:
            for _ in range(20):
                if self.host.calls:
                    break
                await asyncio.sleep(0.01)
            self.assertEqual(self.host.calls, [('approve', self.sid, 'peer')])
            self.event('paired')
            self.event('message', id=str(uuid.uuid4()), text='browser request')
            self.assertEqual(cli('read')['messages'][0]['text'], 'browser request')
            reply = cli('reply', 'different operator text')
            for _ in range(50):
                if len(self.host.calls) == 2:
                    break
                await asyncio.sleep(0.01)
            self.assertEqual(self.host.calls[1][3], 'different operator text')
            self.assertEqual(cli('read')['messages'][1]['status'], 'sent')
            self.event('ack', id=reply['queued'])
            self.assertEqual(cli('read')['messages'][1]['status'], 'received by peer')
        finally:
            self.inbox.shutdown()
            consumer.cancel()
            await asyncio.gather(consumer, return_exceptions=True)

if __name__ == '__main__':
    unittest.main()
