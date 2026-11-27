import hashlib
import json
import os
from pathlib import Path
from types import SimpleNamespace
import sys
import tempfile
import unittest
from unittest import mock

import dependency_manager as manager


class DependencyManifestTests(unittest.TestCase):
    def test_manifest_has_pinned_official_hashes_and_supported_target(self):
        manifest = manager.load_manifest()
        target = manager.target_for(manifest, SimpleNamespace(major=3, minor=13), "Windows", "AMD64")
        self.assertEqual(target["abi"], "cp313")
        self.assertEqual(len(target["packages"]), 8)
        self.assertTrue(all(p["url"].startswith("https://files.pythonhosted.org/") for p in target["packages"]))
        self.assertTrue(all(len(p["sha256"]) == 64 for p in target["packages"]))

    def test_wrong_abi_and_platform_are_not_installable(self):
        manifest = manager.load_manifest()
        self.assertIsNone(manager.target_for(manifest, SimpleNamespace(major=3, minor=14), "Windows", "AMD64"))
        self.assertIsNone(manager.target_for(manifest, SimpleNamespace(major=3, minor=13), "Linux", "x86_64"))

    def test_unapproved_download_host_is_rejected(self):
        manifest = manager.load_manifest()
        manifest["supportedTargets"][0]["packages"][0]["url"] = "https://evil.example/package.whl"
        with self.assertRaisesRegex(manager.InstallError, "approved PyPI"):
            manager.load_manifest_from_data(manifest)


class DependencyTransactionTests(unittest.TestCase):
    def setUp(self):
        # Transaction tests exercise a supported target without requiring the
        # CI host itself to run Windows CPython 3.13 or installing any wheels.
        target = manager.target_for(manager.load_manifest(), SimpleNamespace(major=3, minor=13), "Windows", "AMD64")
        self.assertIsNotNone(target)
        patcher = mock.patch.object(manager, "target_for", return_value=target)
        patcher.start()
        self.addCleanup(patcher.stop)

    def test_active_environment_health_runs_import_smoke(self):
        with tempfile.TemporaryDirectory() as temporary:
            python = manager._python_in(Path(temporary) / "active")
            python.parent.mkdir(parents=True)
            python.touch()
            with mock.patch.object(manager.subprocess, "run", return_value=mock.Mock(returncode=0)) as run:
                self.assertTrue(manager._active_healthy(python.parent.parent))
            self.assertEqual(run.call_args.args[0][0], str(python))

    def test_active_environment_with_missing_imports_is_unhealthy(self):
        with tempfile.TemporaryDirectory() as temporary:
            python = manager._python_in(Path(temporary) / "active")
            python.parent.mkdir(parents=True)
            python.touch()
            with mock.patch.object(manager.subprocess, "run", return_value=mock.Mock(returncode=1)):
                self.assertFalse(manager._active_healthy(python.parent.parent))

    def test_inspect_marks_importable_managed_environment_ready(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "3d-runtime"
            python = manager._python_in(root / "active")
            python.parent.mkdir(parents=True)
            python.touch()
            with mock.patch.object(manager.subprocess, "run", return_value=mock.Mock(returncode=0)):
                self.assertEqual(manager.inspect(root, manager.load_manifest())["state"], "ready")

    def test_healthy_active_environment_is_preserved_during_backup_recovery(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "3d-runtime"
            active, backup = root / "active", root / "previous"
            active.mkdir(parents=True)
            backup.mkdir()
            (active / "keep.txt").write_text("working runtime", encoding="utf-8")
            (backup / "keep.txt").write_text("older runtime", encoding="utf-8")
            with mock.patch.object(manager, "_active_healthy", return_value=True), \
                 mock.patch.object(manager, "_download", side_effect=OSError("offline")):
                with self.assertRaisesRegex(OSError, "offline"):
                    manager.install(root, manager.load_manifest())
            self.assertEqual((active / "keep.txt").read_text(encoding="utf-8"), "working runtime")
            self.assertFalse(backup.exists())

    def test_wrong_wheel_hash_is_removed(self):
        package = {"name": "test", "url": "https://files.pythonhosted.org/test.whl", "size": 3, "sha256": "0" * 64}

        class Response:
            def __enter__(self): return self
            def __exit__(self, *_args): return False
            def geturl(self): return package["url"]
            def read(self, _size):
                if getattr(self, "sent", False): return b""
                self.sent = True
                return b"abc"

        with tempfile.TemporaryDirectory() as temporary:
            target = Path(temporary) / "wheel.whl"
            with mock.patch.object(manager.urllib.request, "urlopen", return_value=Response()):
                with self.assertRaisesRegex(manager.InstallError, "SHA-256"):
                    manager._download(package, target)
            self.assertFalse(target.exists())

    def test_low_storage_stops_before_download(self):
        with tempfile.TemporaryDirectory() as temporary:
            usage = SimpleNamespace(total=100, used=100, free=0)
            with mock.patch.object(manager.shutil, "disk_usage", return_value=usage), \
                 mock.patch.object(manager, "_download") as download:
                with self.assertRaisesRegex(manager.InstallError, "Insufficient storage"):
                    manager.install(Path(temporary), manager.load_manifest())
            download.assert_not_called()

    def test_cancelled_install_preserves_active_environment(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "3d-runtime"
            active = root / "active"
            active.mkdir(parents=True)
            sentinel = active / "keep.txt"
            sentinel.write_text("previous", encoding="utf-8")
            cancel = root / "install.cancel"
            cancel.touch()
            with self.assertRaisesRegex(manager.InstallError, "cancelled"):
                manager.install(root, manager.load_manifest(), cancel_file=cancel)
            self.assertEqual(sentinel.read_text(encoding="utf-8"), "previous")
            self.assertFalse((root / "install.lock").exists())

    def test_concurrent_install_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            lock = root / "install.lock"
            lock.mkdir()
            (lock / "owner.json").write_text(json.dumps({"pid": os.getpid()}), encoding="utf-8")
            with self.assertRaisesRegex(manager.InstallError, "already running"):
                manager.install(root, manager.load_manifest())

    def test_stale_installer_lock_is_recovered(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            lock = root / "install.lock"
            lock.mkdir()
            (lock / "owner.json").write_text(json.dumps({"pid": 2_000_000_000}), encoding="utf-8")
            with mock.patch.object(manager, "_download", side_effect=OSError("simulated offline")):
                with self.assertRaisesRegex(OSError, "simulated offline"):
                    manager.install(root, manager.load_manifest())
            self.assertFalse(lock.exists())

    def test_offline_download_preserves_existing_active_environment(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "active").mkdir()
            (root / "active" / "previous.txt").write_text("ok", encoding="utf-8")
            with mock.patch.object(manager, "_download", side_effect=OSError("offline")):
                with self.assertRaisesRegex(OSError, "offline"):
                    manager.install(root, manager.load_manifest())
            self.assertEqual((root / "active" / "previous.txt").read_text(encoding="utf-8"), "ok")
            self.assertFalse((root / "install.lock").exists())

    def test_interrupted_swap_recovers_previous_environment(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            backup = root / "previous"
            backup.mkdir()
            (backup / "keep.txt").write_text("previous active", encoding="utf-8")
            with mock.patch.object(manager, "_download", side_effect=OSError("simulated offline")):
                with self.assertRaisesRegex(OSError, "simulated offline"):
                    manager.install(root, manager.load_manifest())
            self.assertEqual((root / "active" / "keep.txt").read_text(encoding="utf-8"), "previous active")
            self.assertFalse(backup.exists())

    def test_interrupted_activation_restores_previous_environment(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            active = root / "active"
            active.mkdir()
            (active / "previous.txt").write_text("old", encoding="utf-8")

            class FakeBuilder:
                def __init__(self, **_kwargs): pass
                def create(self, target):
                    python = manager._python_in(Path(target))
                    python.parent.mkdir(parents=True, exist_ok=True)
                    python.touch()

            def fake_run(command, **_kwargs):
                if "--worker-smoke" in command:
                    raise RuntimeError("simulated restart interruption")
                return mock.Mock(stdout="", stderr="")

            with mock.patch.object(manager, "_download", side_effect=lambda _p, path, *_args: path.write_bytes(b"wheel")), \
                 mock.patch.object(manager.venv, "EnvBuilder", FakeBuilder), \
                 mock.patch.object(manager.subprocess, "run", side_effect=fake_run), \
                 mock.patch.object(manager, "_emit"):
                with self.assertRaisesRegex(RuntimeError, "restart interruption"):
                    manager.install(root, manager.load_manifest())
            self.assertEqual((active / "previous.txt").read_text(encoding="utf-8"), "old")
            self.assertFalse((root / "install.lock").exists())


if __name__ == "__main__":
    unittest.main()
