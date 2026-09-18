"""Private local operator spool. Incoming text is data; never dispatches work."""
import argparse
import asyncio
import fcntl
import json
import os
from pathlib import Path
import stat
import sys
import time
import uuid

# 512 messages × (2048 UTF-8 bytes × worst-case six-byte JSON escape),
# plus bounded UUID/peer/status metadata. Keep the protocol/message limits intact.
LIMIT = 7_000_000
COMMAND_TTL = 300


def check(path, directory=False):
    info = path.lstat()
    if (info.st_uid != os.getuid() or info.st_mode & 0o077
            or not (stat.S_ISDIR(info.st_mode) if directory else stat.S_ISREG(info.st_mode))
            or (not directory and info.st_nlink != 1)):
        raise ValueError('unsafe private path')


def read(path, limit=LIMIT):
    check(path.parent, True)
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(fd, 'rb') as stream:
        info = os.fstat(stream.fileno())
        if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid() or info.st_mode & 0o077 or info.st_nlink != 1:
            raise ValueError('unsafe private file')
        raw = stream.read(limit + 1)
    if len(raw) > limit:
        raise ValueError('file limit')
    return json.loads(raw)


def atomic(path, value):
    check(path.parent, True)
    raw = json.dumps(value, ensure_ascii=False).encode()
    if len(raw) > LIMIT:
        raise ValueError('file limit')
    temporary = path.parent / ('.tmp-' + str(uuid.uuid4()))
    fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    try:
        with os.fdopen(fd, 'wb') as stream:
            stream.write(raw)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def valid_id(value):
    if not isinstance(value, str) or str(uuid.UUID(value)) != value:
        raise ValueError('invalid identifier')
    return value


def text(value):
    if not isinstance(value, str) or not 0 < len(value.encode()) <= 2048:
        raise ValueError('message must contain 1–2048 UTF-8 bytes')
    return value


class Spool:
    def __init__(self, root):
        self.root = Path(root).absolute()
        # Refuse symlinks in every ancestor, including the supplied root.
        for parent in [self.root, *self.root.parents]:
            if parent.is_symlink():
                raise ValueError('symlink path')
        self.root.mkdir(mode=0o700, exist_ok=True)
        check(self.root, True)
        self.commands = self.root / 'commands'
        self.commands.mkdir(mode=0o700, exist_ok=True)
        check(self.commands, True)

    def lock(self, name):
        path = self.root / name
        fd = os.open(path, os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW | os.O_NONBLOCK, 0o600)
        try:
            check(path)
            fcntl.flock(fd, fcntl.LOCK_EX | (fcntl.LOCK_NB if name == 'runner.lock' else 0))
            return os.fdopen(fd, 'r+')
        except BaseException:
            os.close(fd)
            raise

    def purge(self, all_files=False):
        # Caller holds queue.lock. Never follow links or interpret temporary files.
        now = time.time()
        for directory in (self.commands, self.root):
            for path in directory.iterdir():
                if directory == self.root and not path.name.startswith('.tmp-'):
                    continue
                check(path)
                if all_files or now - path.stat().st_mtime >= COMMAND_TTL:
                    path.unlink()

    def fence(self):
        with self.lock('queue.lock'):
            atomic(self.root / 'inbox.json', {'run': None, 'active': False,
                   'updated': time.time(), 'sessions': {}})
            self.purge(all_files=True)

    def claim(self):
        with self.lock('queue.lock'):
            self.purge()
            for path in sorted(self.commands.iterdir()):
                if path.suffix != '.json':
                    continue
                try:
                    valid_id(path.stem)
                except ValueError:
                    continue
                try:
                    command = read(path, 16384)
                except (ValueError, OSError):
                    path.unlink(missing_ok=True)
                    continue
                path.unlink()  # At-most-once claim; no retry after uncertain send.
                if not isinstance(command, dict) or command.get('id') != path.stem:
                    continue
                return command
        return None

    def snapshot(self):
        return read(self.root / 'inbox.json')

    def submit(self, action, session_id, peer_id, body=None):
        valid_id(session_id)
        with self.lock('queue.lock'):
            self.purge()
            snapshot = self.snapshot()
            session = snapshot['sessions'].get(session_id)
            if (not snapshot.get('active') or not 0 <= time.time() - snapshot['updated'] <= 10 or not session or session['peer_id'] != peer_id
                    or time.time() >= session['expires'] or session['state'] == 'closed'):
                raise ValueError('session unavailable')
            if action not in ('approve', 'reply') or (action == 'reply' and session['state'] != 'paired'):
                raise ValueError('invalid operation')
            if len(list(self.commands.iterdir())) >= 128:
                raise ValueError('outbox full')
            command = {'run': snapshot['run'], 'action': action, 'session_id': session_id,
                       'peer_id': peer_id, 'id': str(uuid.uuid4()), 'created': time.time()}
            if action == 'reply':
                command['text'] = text(body)
            atomic(self.commands / (command['id'] + '.json'), command)
            return command['id']


class Inbox:
    def __init__(self, spool, host):
        self.spool, self.host = spool, host
        self.run = str(uuid.uuid4())
        self.sessions = {}
        self.active = True
        # Fence the old run and purge plaintext before publishing new authority.
        spool.fence()
        self.save()

    def save(self):
        now = time.time()
        self.sessions = {sid: session for sid, session in self.sessions.items() if session['expires'] > now}
        atomic(self.spool.root / 'inbox.json', {'run': self.run, 'active': self.active, 'updated': now, 'sessions': self.sessions})

    def shutdown(self):
        self.active = False
        self.sessions.clear()
        self.spool.fence()

    def accept(self, event):
        """Contain invalid/stale session events; storage failures remain observable."""
        if not isinstance(event, dict):
            return False
        try:
            self.event(event)
            return True
        except (ValueError, KeyError, TypeError):
            session = self.sessions.get(event.get('session_id'))
            if session and session['peer_id'] == event.get('peer_id'):
                session['state'] = 'closed'
                session['closed_at'] = time.time()
                for message in session['messages']:
                    if message['status'] in ('queued', 'sent'):
                        message['status'] = 'not confirmed'
                self.save()
            return False

    def event(self, event):
        if not self.active:
            return
        sid = valid_id(event['session_id'])
        peer = event['peer_id']
        if not isinstance(peer, str) or not 1 <= len(peer) <= 256:
            raise ValueError('invalid peer')
        kind = event['type']
        self.sessions = {key: value for key, value in self.sessions.items() if value['expires'] > time.time()}
        session = self.sessions.get(sid)
        if kind == 'closed' and (not session or session['state'] == 'closed'):
            self.save()
            return
        if kind == 'pending':
            if not session and len(self.sessions) >= 4:
                oldest = min((key for key, value in self.sessions.items() if value['state'] == 'closed'),
                             key=lambda key: self.sessions[key]['closed_at'], default=None)
                if oldest is not None:
                    del self.sessions[oldest]
            if session or len(self.sessions) >= 4:
                raise ValueError('session capacity or replay')
            self.sessions[sid] = {'peer_id': peer, 'state': 'pending', 'expires': time.time() + 300, 'messages': []}
        else:
            if not session or session['peer_id'] != peer or session['state'] == 'closed' or session['expires'] <= time.time():
                raise ValueError('stale event')
            if kind == 'paired' and session['state'] == 'pending':
                session['state'] = 'paired'
                session['expires'] = time.time() + 3600
            elif kind == 'closed':
                session['state'] = 'closed'
                session['closed_at'] = time.time()
                for message in session['messages']:
                    if message['status'] in ('queued', 'sent'):
                        message['status'] = 'not confirmed'
            elif kind == 'message' and session['state'] == 'paired':
                mid = valid_id(event['id'])
                if len(session['messages']) >= 128 or any(m['id'] == mid for m in session['messages']):
                    raise ValueError('message capacity or replay')
                session['messages'].append({'id': mid, 'text': text(event['text']), 'direction': 'in', 'status': 'received by host'})
            elif kind == 'ack' and session['state'] == 'paired':
                message = next((m for m in session['messages'] if m['id'] == event['id'] and m['direction'] == 'out' and m['status'] in ('queued', 'sent')), None)
                if not message:
                    raise ValueError('unknown receipt')
                message['status'] = 'received by peer'
            else:
                raise ValueError('unapproved or invalid event')
        self.save()

    async def command(self, command):
        session = self.sessions.get(command.get('session_id'))
        if (not self.active or command.get('run') != self.run or not session or session['peer_id'] != command.get('peer_id')
                or session['state'] == 'closed' or session['expires'] <= time.time()
                or not 0 <= time.time() - command['created'] <= 300):
            raise ValueError('stale command')
        sid = command['session_id']
        if command['action'] == 'approve' and session['state'] == 'pending':
            await self.host.approve(sid, session['peer_id'])
        elif command['action'] == 'reply' and session['state'] == 'paired':
            mid, body = valid_id(command['id']), text(command['text'])
            if len(session['messages']) >= 128 or any(m['id'] == mid for m in session['messages']):
                raise ValueError('outbox full or duplicate')
            message = {'id': mid, 'text': body, 'direction': 'out', 'status': 'queued'}
            session['messages'].append(message)
            try:
                await self.host.send(sid, mid, body)
                if message['status'] == 'queued':
                    message['status'] = 'sent'
            except Exception:
                if message['status'] != 'received by peer':
                    message['status'] = 'not confirmed'
                raise
            finally:
                self.save()
        else:
            raise ValueError('invalid command')


async def consume_commands(inbox):
    """Production and loopback fixture share the same serialized command consumer."""
    while inbox.active:
        for _ in range(128):
            command = inbox.spool.claim()
            if command is None:
                break
            try:
                await asyncio.wait_for(inbox.command(command), 15)
            except Exception:
                atomic(inbox.spool.root / 'last-command.json', {
                    'id': command.get('id'), 'state': 'rejected or not confirmed'})
        inbox.save()
        await asyncio.sleep(0.2)


async def receive_events(inbox):
    while inbox.active:
        raw = await inbox.host.next_event()
        if len(raw) > 16384:
            raise ValueError('event limit')
        inbox.accept(json.loads(raw))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--root', required=True)
    parser.add_argument('action', choices=['list', 'read', 'approve', 'reply'])
    parser.add_argument('--session')
    parser.add_argument('--peer')
    parser.add_argument('--text-file', type=Path)
    args = parser.parse_args()
    spool = Spool(args.root)
    if args.action == 'list':
        snapshot = spool.snapshot()
        print(json.dumps([{**{k: v for k, v in value.items() if k != 'messages'}, 'session_id': key}
                          for key, value in snapshot['sessions'].items() if value['expires'] > time.time()]))
    elif args.action == 'read':
        value = spool.snapshot()['sessions'][valid_id(args.session)]
        if value['peer_id'] != args.peer or value['expires'] <= time.time():
            raise ValueError('wrong peer or expired session')
        print(json.dumps(value, ensure_ascii=True))
    else:
        body = None
        if args.action == 'reply':
            if args.text_file:
                check(args.text_file)
                with args.text_file.open('rb') as stream:
                    body = stream.read(2049).decode('utf-8')
            else:
                body = sys.stdin.buffer.read(2049).decode('utf-8')
        print(json.dumps({'queued': spool.submit(args.action, args.session, args.peer, body)}))


if __name__ == '__main__':
    try:
        main()
    except Exception:
        print('Operator command rejected.', file=sys.stderr)
        raise SystemExit(1)
