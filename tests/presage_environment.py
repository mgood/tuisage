"""Exercise Presage queries without launching providers or operational commands."""
import json
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile
import unittest

BINARY = Path(sys.argv.pop(1)).resolve()


class EnvironmentTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="presage-environment-")
        self.root = Path(self.temporary.name).resolve()
        self.home = self.root / "home"
        self.home.mkdir()
        self.bin = self.root / "installed with spaces"
        self.bin.mkdir()
        self.command = self.bin / "presage-environment-demo"
        self.command.write_text("#!/bin/sh\nexit 99\n")
        self.command.chmod(0o755)
        self.spec = self.root / "demo.usage.kdl"
        self.spec.write_text('name "presage-environment-demo"\ncmd "run"\n')
        self.env = {"HOME": str(self.home), "PATH": "/usr/bin:/bin"}
        self.user = (self.home / "Library/Application Support/TuiSage/commands"
                     if sys.platform == "darwin" else self.home / ".local/share/tuisage/commands")

    def tearDown(self):
        self.temporary.cleanup()

    def document(self, directory, selector=""):
        directory.mkdir(parents=True, exist_ok=True)
        suffix = f".{selector}" if selector else ""
        path = directory / f"{self.command.name}{suffix}.tuisage.kdl"
        path.write_text("version 1\n")
        return path

    def query(self, *options, command=None):
        result = subprocess.run([str(BINARY), "--presage", *options,
            "--cmd", shlex.quote(str(command or self.command)), "--spec-file", str(self.spec)],
            env=self.env, cwd=self.root, capture_output=True, text=True, timeout=5)
        self.assertEqual(result.returncode, 0, result.stderr)
        return json.loads(result.stdout)

    def test_user_override_and_sidecar_with_symlinked_command(self):
        sidecar = self.document(self.bin, "one")
        link = self.root / "alias"
        link.symlink_to(self.command)
        self.env["PATH"] = "/nonexistent"
        self.assertEqual(self.query("--selector", "one", command=link)["path"], str(sidecar))
        override = self.document(self.user, "one")
        result = self.query("--selector", "one", command=link)
        self.assertEqual(result["path"], str(override))
        self.assertEqual(result["selectors"], {"one": str(override)})

    def test_explicit_directory_precedes_cascade_and_enumerates_selectors(self):
        self.document(self.user, "one")
        self.document(self.bin, "two")
        directory = self.root / "application documents"
        selected = self.document(directory, "one")
        result = self.query("--presage", str(directory), "--selector", "one")
        self.assertEqual(result["path"], str(selected))
        self.assertEqual(result["selectors"]["one"], str(selected))
        self.assertIn("two", result["selectors"])
        self.assertIsNone(self.query("--presage", str(directory), "--selector", "missing")["path"])
        self.assertEqual(self.query("--presage", str(selected))["path"], str(selected))

    def test_missing_selector_does_not_fall_back_to_generic(self):
        self.document(self.bin)
        self.assertIsNone(self.query("--selector", "missing")["path"])

    def test_invalid_home_does_not_search_working_directory(self):
        self.document(self.root / "Library/Application Support/TuiSage/commands")
        self.document(self.root / ".local/share/tuisage/commands")
        self.document(self.root / "relative/Library/Application Support/TuiSage/commands")
        self.document(self.root / "relative/.local/share/tuisage/commands")
        for value in (None, "", "relative"):
            with self.subTest(home=value):
                self.env.pop("HOME", None)
                if value is not None:
                    self.env["HOME"] = value
                self.assertIsNone(self.query()["path"])

    def test_read_only_selected_document_and_missing_explicit_path(self):
        explicit = self.document(self.bin)
        explicit.chmod(0o444)
        before = explicit.read_bytes()
        self.assertEqual(self.query()["path"], str(explicit))
        self.assertEqual(explicit.read_bytes(), before)
        result = subprocess.run([str(BINARY), "--presage", str(self.root / "absent"),
            "--cmd", shlex.quote(str(self.command)), "--spec-file", str(self.spec)],
            env=self.env, capture_output=True, text=True, timeout=5)
        self.assertNotEqual(result.returncode, 0)

    @unittest.skipUnless(sys.platform.startswith("linux"), "Linux runtime check")
    def test_relative_xdg_data_home_falls_back_to_home(self):
        override = self.document(self.user)
        self.env["XDG_DATA_HOME"] = "relative"
        self.assertEqual(self.query()["path"], str(override))


if __name__ == "__main__":
    unittest.main()
