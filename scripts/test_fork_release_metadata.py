from pathlib import Path
import os
import json
import subprocess
import sys
import tempfile
import unittest

from scripts.fork_release_metadata import metadata


class ForkReleaseMetadataTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="herdr-fork-metadata-")
        self.root = Path(self.temporary.name).resolve()
        self.assertTrue(self.root.is_relative_to(Path(tempfile.gettempdir()).resolve()))
        self.addCleanup(self.temporary.cleanup)
        self.git("init", "-q")
        self.git("config", "user.name", "Fixture")
        self.git("config", "user.email", "fixture@example.com")
        self.git("config", "commit.gpgsign", "false")
        (self.root / "Cargo.toml").write_text('[package]\nversion = "0.9.5"\n', encoding="utf-8")
        directory = self.root / "src/protocol"
        directory.mkdir(parents=True)
        (directory / "wire.rs").write_text("pub const PROTOCOL_VERSION: u32 = 22;\n")
        (directory / "endpoint.rs").write_text("pub const ENDPOINT_PROTOCOL_GENERATION: u32 = 1;\n")
        self.git("add", ".")
        self.git("commit", "-qm", "feat: fixture")

    def git(self, *args):
        env = dict(os.environ, GIT_AUTHOR_DATE="2026-11-11T07:00:00Z",
                   GIT_COMMITTER_DATE="2026-11-11T07:00:00Z")
        return subprocess.check_output(["git", "-C", str(self.root), *args], env=env, text=True).strip()

    def test_tag_is_shared_identity_and_display_version(self):
        tag = "fork-v0.9.5-preview.2026-11-11-086fa74f"
        values = metadata(self.root, "", tag)
        self.assertEqual(values["tag"], tag)
        self.assertEqual(values["build_id"], tag)
        self.assertEqual(values["channel"], "preview")
        self.assertEqual(values["commit"], self.git("rev-parse", "HEAD"))
        self.assertEqual(values["base_version"], "0.9.5")
        self.assertEqual(values["protocol"], "22")
        self.assertEqual(values["endpoint_generation"], "1")

    def test_manual_and_tag_runs_resolve_identical_metadata(self):
        version = "0.9.5-preview.2026-11-11-086fa74f"
        self.assertEqual(metadata(self.root, version, "master"),
                         metadata(self.root, "", f"fork-v{version}"))
        self.assertEqual(metadata(self.root, version, "master")["built_at"], "2026-11-11T07:00:00Z")

    def test_existing_tag_accepts_its_commit_and_rejects_a_new_one(self):
        self.git("tag", "fork-v0.9.5")
        metadata(self.root, "0.9.5", "master")
        (self.root / "changed").write_text("new source")
        self.git("add", ".")
        self.git("commit", "-qm", "fix: fixture")
        with self.assertRaisesRegex(ValueError, "different commit"):
            metadata(self.root, "0.9.5", "master")

    def test_invalid_versions_and_untagged_runs_are_rejected(self):
        for version, ref in [("", "master"), ("0.9.5\ncommit=other", "master"),
                             ("0.9.5;echo bad", "master"), ("$(echo bad)", "master")]:
            with self.subTest(version=version, ref=ref), self.assertRaises(ValueError):
                metadata(self.root, version, ref)

    def test_cli_exports_the_same_metadata_for_github_actions(self):
        script = Path(__file__).with_name("fork_release_metadata.py").resolve()
        output = self.root / "github-output"
        artifact = self.root / "metadata.json"
        version = "0.9.5-preview.2026-11-11-086fa74f"
        subprocess.check_call(
            [sys.executable, str(script), "--github-output", str(output), "--json-output", str(artifact)],
            cwd=self.root, env=dict(os.environ, FORK_RELEASE_VERSION=version,
                                   GITHUB_REF_NAME="master"),
        )
        exported = dict(line.split("=", 1) for line in output.read_text(encoding="utf-8").splitlines())
        self.assertEqual(exported, metadata(self.root, version, "master"))
        self.assertEqual(json.loads(artifact.read_text(encoding="utf-8")), exported)


if __name__ == "__main__":
    unittest.main()
