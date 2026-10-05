"""PTY checks for submit bindings, clickable actions, and terminal cleanup."""

import fcntl
import os
import pty
import select
import struct
import sys
import tempfile
import termios
import time
from pathlib import Path


BINARY = Path(sys.argv[1] if len(sys.argv) > 1 else "target/debug/tuisage").resolve()


def run_case(root, name, keys, keymap=None, environment=None, expect_submit=True):
    marker = root / f"{name}.ran"
    child = root / f"{name}.sh"
    child.write_text(f"#!/bin/sh\ntouch '{marker}'\n")
    child.chmod(0o755)
    spec = root / f"{name}.usage.kdl"
    spec.write_text(f'name "fake"\nbin "{child}"\ncmd "go" {{}}\n')

    command = [str(BINARY), "--spec-file", str(spec)]
    if keymap is not None:
        command += ["--keymap", str(keymap)]

    pid, fd = pty.fork()
    if pid == 0:
        fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 120, 0, 0))
        os.environ.update(environment or {})
        os.execv(str(BINARY), command)
    output = bytearray()
    started = time.monotonic()
    sent_keys = False
    cleanup_sent = False
    exit_status = None

    while time.monotonic() - started < 8:
        readable, _, _ = select.select([fd], [], [], 0.05)
        if readable:
            try:
                output.extend(os.read(fd, 65536))
            except OSError:
                break

        if not sent_keys and b"\x1b[?1049h" in output:
            time.sleep(0.15)
            os.write(fd, keys)
            sent_keys = True

        if expect_submit and marker.exists() and not cleanup_sent:
            time.sleep(0.2)
            os.write(fd, b"\x1b")
            time.sleep(0.15)
            os.write(fd, b"q")
            cleanup_sent = True
        elif sent_keys and not expect_submit and not cleanup_sent:
            time.sleep(0.25)
            os.write(fd, b"q")
            cleanup_sent = True

        waited, status = os.waitpid(pid, os.WNOHANG)
        if waited:
            exit_status = status
            break

    if exit_status is None:
        os.kill(pid, 9)
        _, exit_status = os.waitpid(pid, 0)
        raise AssertionError(f"{name} timed out; output={output.decode(errors='replace')}")

    assert os.waitstatus_to_exitcode(exit_status) == 0, name
    assert marker.exists() == expect_submit, name
    assert b"\x1b[?1049l" in output, f"alternate screen not restored: {name}"
    assert b"\x1b[?1000l" in output, f"mouse mode not restored: {name}"
    assert b"\x1b[<1u" in output, f"keyboard mode not restored: {name}"


def main():
    with tempfile.TemporaryDirectory(prefix="tuisage-keymap-") as directory:
        root = Path(directory)
        sample = root / "keymap.toml"
        sample.write_text(
            '[bindings]\n"ctrl+r" = "submit"\n"enter" = "submit"\n'
            '"keypad-enter" = "submit"\n"ctrl+c" = "cancel"\n"q" = "cancel"\n'
        )
        for name, keys in [
            ("ctrl-r", b"\x12"),
            ("enter", b"\r"),
            ("keypad-enter", b"\x1b[57414u"),
        ]:
            run_case(root, name, keys, sample)

        no_default_keymap = {"XDG_CONFIG_HOME": str(root / "empty-config")}
        for name, keys in [
            ("default-enter", b"\r"),
            ("default-keypad-enter", b"\x1b[57414u"),
        ]:
            run_case(
                root,
                name,
                keys,
                environment=no_default_keymap,
                expect_submit=False,
            )

        config = root / "config"
        default_keymap = config / "tuisage/keymap.toml"
        default_keymap.parent.mkdir(parents=True)
        default_keymap.write_text('[bindings]\n"alt+s" = "submit"\n')
        run_case(
            root,
            "xdg-path",
            b"\x1bs",
            environment={"XDG_CONFIG_HOME": str(config)},
        )

        explicit = root / "explicit.toml"
        explicit.write_text('[bindings]\n"alt+t" = "submit"\n')
        run_case(
            root,
            "explicit-path",
            b"\x1bt",
            explicit,
            environment={"XDG_CONFIG_HOME": str(config)},
        )

        unbound = root / "unbound.toml"
        unbound.write_text('[bindings]\n"ctrl+r" = "unbound"\n')
        run_case(root, "unbound", b"\x12", unbound, expect_submit=False)

        run_case(root, "mouse-action", b"\x1b[<0;48;30M", sample)

    print("PTY keymap, submit, mouse-action, and terminal cleanup checks passed")


if __name__ == "__main__":
    main()
