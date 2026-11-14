from copy import deepcopy
import hashlib
from pathlib import Path
import tempfile
import unittest

from scripts.fork_release_manifests import ASSETS, manifests


def fixture():
    return ({
        "tag": "fork-v0.9.5-preview.2026-11-14-abcdef12",
        "build_id": "fork-v0.9.5-preview.2026-11-14-abcdef12",
        "base_version": "0.9.5", "commit": "a" * 40,
        "built_at": "2026-11-14T00:00:00Z", "protocol": "22", "endpoint_generation": "1",
    }, {
        "channel": "preview", "build_id": "fork-v0.9.5-preview.2026-11-11-162dd3cb",
        "base_version": "0.9.5", "commit": "162dd3cb46adb35c5941b0f98758d51e33ea0512",
        "built_at": "2026-11-11T11:56:00Z", "protocol": 22,
        "assets": {"linux-x86_64": {"url": "https://example.test/old", "sha256": "b" * 64}},
        "builds": {"fork-v0.9.4": {"commit": "c" * 40, "assets": {}}},
    })


class ForkReleaseManifestTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="herdr-fork-feeds-")
        self.root = Path(self.temporary.name).resolve()
        self.assertTrue(self.root.is_relative_to(Path(tempfile.gettempdir()).resolve()))
        self.addCleanup(self.temporary.cleanup)
        for name in ASSETS.values():
            directory = self.root / name
            directory.mkdir()
            (directory / name).write_bytes(name.encode())

    def test_both_feeds_share_all_five_artifacts_checksums_and_identity(self):
        metadata, previous = fixture()
        preview, installer = manifests(metadata, "zubo334/herdr", self.root, previous)
        self.assertEqual(set(preview["assets"]), set(ASSETS))
        self.assertEqual(installer["channel"], "preview")
        self.assertEqual(installer["version"], metadata["tag"])
        self.assertEqual(preview["build_id"], metadata["tag"])
        self.assertEqual(preview["assets"]["windows-x86_64"]["format"], "zip")
        for target, filename in ASSETS.items():
            asset = preview["assets"][target]
            self.assertEqual(asset["sha256"], hashlib.sha256(filename.encode()).hexdigest())
            self.assertEqual(installer["assets"][target], asset["url"])
            self.assertEqual(installer["sha256"][target], asset["sha256"])
            self.assertIn(f'/releases/download/{metadata["tag"]}/', asset["url"])

    def test_previous_builds_remain_available_to_existing_ssh_clients(self):
        metadata, previous = fixture()
        before = deepcopy(previous)
        preview, _ = manifests(metadata, "zubo334/herdr", self.root, previous)
        old = preview["builds"][previous["build_id"]]
        self.assertEqual(old["assets"], previous["assets"])
        self.assertEqual(old["commit"], previous["commit"])
        self.assertIn("fork-v0.9.4", preview["builds"])
        self.assertIn(metadata["tag"], preview["builds"])
        self.assertEqual(previous, before)

    def test_foreign_history_missing_artifact_or_changed_identity_fail(self):
        metadata, previous = fixture()
        previous["build_id"] = "upstream"
        with self.assertRaisesRegex(ValueError, "not a fork preview"):
            manifests(metadata, "zubo334/herdr", self.root, previous)
        metadata, previous = fixture()
        previous["build_id"] = metadata["tag"]
        with self.assertRaisesRegex(ValueError, "different commit"):
            manifests(metadata, "zubo334/herdr", self.root, previous)
        metadata, previous = fixture()
        (self.root / "herdr-linux-x86_64/herdr-linux-x86_64").unlink()
        with self.assertRaises(FileNotFoundError):
            manifests(metadata, "zubo334/herdr", self.root, previous)


if __name__ == "__main__":
    unittest.main()
