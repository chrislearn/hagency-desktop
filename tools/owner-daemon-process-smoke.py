#!/usr/bin/env python3
"""Exercise a built production daemon in a private, unenrolled temporary home.

No service installation, accounts, credentials, provider calls or live state.
Enrollment and encrypted execution have separate integration fixtures.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import signal
import socket
import stat
import struct
import subprocess
import tempfile
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    binary, output = args.binary.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    checks = []
    children = []

    def check(name, condition):
        checks.append({'check': name, 'passed': bool(condition)})
        assert condition, name

    with tempfile.TemporaryDirectory(prefix='hg-ipc-', dir='/tmp') as tmp:
        home = Path(tmp) / 'daemon'
        sockpath = home / 'control.sock'

        def connect():
            sock = socket.socket(socket.AF_UNIX)
            sock.settimeout(5)
            sock.connect(str(sockpath))
            return sock

        def receive(sock):
            def exact(size):
                data = b''
                while len(data) < size:
                    chunk = sock.recv(size - len(data))
                    if not chunk:
                        raise EOFError('socket closed')
                    data += chunk
                return data
            size, = struct.unpack('>I', exact(4))
            assert 0 < size <= 4 * 1024 * 1024
            return json.loads(exact(size))

        def call(sock, request):
            data = json.dumps(request).encode()
            sock.sendall(struct.pack('>I', len(data)) + data)
            reply = receive(sock)
            assert reply['version'] == 1
            assert reply['requestId'] == request['requestId']
            return reply

        def request(i, kind='hello', **fields):
            return dict(version=1, requestId='smoke-' + str(i),
                        command={'type': kind}, **fields)

        def start(index):
            log = (output / ('daemon-' + str(index) + '.log')).open('wb')
            proc = subprocess.Popen([str(binary), 'daemon', 'run', '--home', str(home)],
                                    stdout=log, stderr=subprocess.STDOUT)
            children.append(proc)
            log.close()
            deadline = time.monotonic() + 20
            while time.monotonic() < deadline:
                assert proc.poll() is None, 'daemon exited before ready'
                try:
                    with connect() as sock:
                        reply = call(sock, request('ready-' + str(index)))
                    return proc, reply
                except (OSError, EOFError):
                    time.sleep(.05)
            raise AssertionError('daemon did not become ready')

        def stop(proc):
            proc.send_signal(signal.SIGTERM)
            check('graceful_exit_' + str(len(checks)), proc.wait(timeout=30) == 0)

        try:
            proc, hello = start(1)
            check('unenrolled_daemon_reachable', hello['result']['ownerId'] is None)
            check('home_private', stat.S_IMODE(home.stat().st_mode) == 0o700)
            check('socket_private', stat.S_IMODE(sockpath.stat().st_mode) == 0o600)
            status = subprocess.run([str(binary), 'daemon', 'status', '--home', str(home)],
                                    capture_output=True, text=True, timeout=15)
            check('production_cli_status', status.returncode == 0 and bool(status.stdout.strip()))
            duplicate = subprocess.run([str(binary), 'daemon', 'run', '--home', str(home)],
                                       capture_output=True, timeout=15)
            check('second_daemon_refused', duplicate.returncode != 0 and proc.poll() is None)
            with connect() as sock:
                wrong_version = request('version')
                wrong_version['version'] = 99
                reply = call(sock, wrong_version)
                check('unsupported_version_rejected', reply['error']['code'] == 'unsupported_version')
            for kind in ['snapshot', 'pauseHosting', 'resumeHosting', 'stopHosting']:
                with connect() as sock:
                    reply = call(sock, request(kind, kind, ownerId='@fixture:invalid.local'))
                    check('unenrolled_' + kind + '_refused', reply['error']['code'] == 'owner_not_adopted')
            with connect() as sock:
                for index in range(32):
                    reply = call(sock, request('cap-' + str(index)))
                    assert 'result' in reply
                check('connection_cap_32_good_requests', True)
                check('connection_cap_closes_stream', sock.recv(1) == b'')
            with connect() as sock:
                check('fresh_stream_after_cap', 'result' in call(sock, request('fresh')))
            with connect() as sock:
                sock.sendall(struct.pack('>I', 65537))
                try:
                    reply = receive(sock)
                    check('oversize_frame_refused', 'error' in reply)
                except (EOFError, ConnectionResetError):
                    check('oversize_frame_closed', True)
            with connect() as sock:
                final = call(sock, request('healthy'))
                check('healthy_after_invalid_inputs', 'result' in final)
            stop(proc)
            check('socket_removed_on_exit', not sockpath.exists())
            proc, restarted = start(2)
            check('restart_retains_revision', restarted['revision'] >= final['revision'])
            check('restart_remains_unenrolled', restarted['result']['ownerId'] is None)
            stop(proc)
        finally:
            for proc in children:
                if proc.poll() is None:
                    proc.kill()
                    proc.wait(timeout=10)
            report = {'binary': str(binary),
                      'sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
                      'checks': checks,
                      'scope': 'Production CLI and IPC; temporary unenrolled home. No enrollment or inference.'}
            (output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'passed': sum(c['passed'] for c in checks), 'total': len(checks)}))


if __name__ == '__main__':
    main()
