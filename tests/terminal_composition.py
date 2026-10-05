"""PTY acceptance checks with stdout redirected independently of the terminal."""
import errno
import fcntl
import json
import os
from pathlib import Path
import select
import struct
import sys
import tempfile
import termios
import time

BINARY = Path(sys.argv[1] if len(sys.argv) > 1 else 'target/debug/tuisage').resolve()

def session(args, keys, expected_status=0, close_when_file_exists=None):
    read_out, write_out = os.pipe()
    pid, terminal = os.forkpty()
    if pid == 0:
        os.close(read_out)
        os.dup2(write_out, 1)
        os.close(write_out)
        os.execv(str(BINARY), [str(BINARY), *args])
    os.close(write_out)
    fcntl.ioctl(terminal, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 80, 0, 0))
    before = termios.tcgetattr(terminal)
    screen = bytearray()
    output = bytearray()
    started = time.monotonic()
    key_steps = keys if isinstance(keys, list) else [(0.1, keys)]
    sent = 0
    ready_at = None
    last_close_sent = None
    status = None
    while time.monotonic() - started < 10:
        for fd in select.select([terminal, read_out], [], [], 0.05)[0]:
            try:
                data = os.read(fd, 65536)
            except OSError as error:
                if error.errno != errno.EIO:
                    raise
                data = b''
            if fd == terminal:
                screen.extend(data)
            else:
                output.extend(data)
        if ready_at is None and b'\x1b[?1049h' in screen:
            ready_at = time.monotonic()
        if ready_at is not None and sent < len(key_steps):
            delay, sequence = key_steps[sent]
            if time.monotonic() - ready_at >= delay:
                os.write(terminal, sequence)
                sent += 1
        if (
            close_when_file_exists
            and close_when_file_exists.exists()
            and (last_close_sent is None or time.monotonic() - last_close_sent >= 0.2)
        ):
            os.write(terminal, b'q')
            last_close_sent = time.monotonic()
        completed, status = os.waitpid(pid, os.WNOHANG)
        if completed:
            break
    else:
        os.kill(pid, 9)
        os.waitpid(pid, 0)
        raise AssertionError('terminal submission did not return within ten seconds')
    for fd in select.select([read_out], [], [], 0.1)[0]:
        output.extend(os.read(read_out, 65536))
    after = termios.tcgetattr(terminal)
    os.close(terminal)
    os.close(read_out)
    assert os.waitstatus_to_exitcode(status) == expected_status, (status, screen.decode(errors='replace'))
    assert b'\x1b[?1049l' in screen, 'alternate screen not restored'
    assert b'\x1b[?1000l' in screen, 'mouse mode not restored'
    assert b'\x1b[?25h' in screen, 'cursor not restored'
    assert before[3] == after[3], 'terminal input flags not restored'
    return bytes(output), bytes(screen)

with tempfile.TemporaryDirectory() as directory:
    root = Path(directory)
    marker = root / 'executed'
    child = root / 'fake'
    child.write_text('#!/bin/sh\nprintf executed\nprintf executed > "' + str(marker) + '"\n')
    child.chmod(0o755)
    spec = root / 'sample.usage.kdl'
    spec.write_text(
        'name "fake"\nbin "' + str(child) + '"\n'
        'cmd "run" {\n  arg "[value]..."\n}\n'
    )
    args = ['--compose', '--spec-file', str(spec)]
    output, _ = session(args, b'\x12')
    assert json.loads(output) == {'executable': str(child), 'argv': ['run']}
    assert output.count(b'\n') == 1
    assert b'\x1b' not in output
    assert not marker.exists(), 'compose executed the child'
    output, _ = session(args, [
        (0.1, b'\t'),
        (0.2, b'\x0e'),
        (0.3, b'\r'),
        (0.4, b'one'),
        (0.5, b'\r'),
        (0.6, b'\x0e'),
        (0.7, b'\r'),
        (0.8, b'two'),
        (0.9, b'\r'),
        (1.0, b'\x12'),
    ])
    assert json.loads(output) == {
        'executable': str(child),
        'argv': ['run', 'one', 'two'],
    }, output
    assert not marker.exists(), 'compose executed the child'
    # Default Tab focus, Return to edit and commit an empty value, Ctrl+R to submit.
    output, _ = session(args, b'\t\r\r\x12')
    assert json.loads(output) == {'executable': str(child), 'argv': ['run', '']}, output
    assert output.count(b'\n') == 1
    assert b'\x1b' not in output
    assert not marker.exists(), 'compose executed the child'
    output, _ = session(args, b'\x03', expected_status=130)
    assert output == b''
    assert not marker.exists()
    output, _ = session(['--spec-file', str(spec)], b'\x12', close_when_file_exists=marker)
    assert output == b''
    assert marker.read_text() == 'executed'

    marker.unlink()
    provider = root / 'reject'
    provider.write_text('#!/bin/sh\ncat >/dev/null\necho rejected >&2\nexit 2\n')
    provider.chmod(0o755)
    for compose, expected_status in [(True, 130), (False, 0)]:
        validation_args = ['--validate', str(provider), '--spec-file', str(spec)]
        if compose:
            validation_args.insert(0, '--compose')
        output, screen = session(
            validation_args,
            [(0.1, b'\x12'), (0.5, b'\x03')],
            expected_status=expected_status,
        )
        assert output == b''
        assert b'Validation failed' in screen
        assert not marker.exists(), 'rejected validation ran the operational command'
print('PTY composition, normal execution, empty argv, cancellation, validation rejection, clean stdout, no unintended execution and terminal restoration passed')
