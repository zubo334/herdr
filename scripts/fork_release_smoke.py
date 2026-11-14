#!/usr/bin/env python3
"""Check public release binaries with no manifest environment overrides."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import urllib.request
import zipfile

PREVIEW_URL = "https://github.com/zubo334/herdr/releases/latest/download/preview.json"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--target", required=True)
    args = parser.parse_args()
    with urllib.request.urlopen(PREVIEW_URL, timeout=60) as response:
        manifest = json.load(response)
    assert manifest["build_id"] == args.tag, manifest["build_id"]
    assert manifest["commit"] == args.commit, manifest["commit"]
    asset = manifest["assets"][args.target]
    with tempfile.TemporaryDirectory(prefix="herdr-fork-public-") as temporary:
        root = Path(temporary).resolve()
        assert root.is_relative_to(Path(tempfile.gettempdir()).resolve())
        with urllib.request.urlopen(asset["url"], timeout=120) as response:
            payload = response.read()
        assert hashlib.sha256(payload).hexdigest() == asset["sha256"]
        if args.target.startswith("windows-"):
            archive = root / "package.zip"
            archive.write_bytes(payload)
            with zipfile.ZipFile(archive) as package:
                assert all((root / name).resolve().is_relative_to(root) for name in package.namelist())
                package.extractall(root)
            executable = root / "herdr.exe"
            assert (root / "conpty/conpty.dll").is_file()
        else:
            executable = root / "herdr"
            executable.write_bytes(payload)
            executable.chmod(0o755)
        digest = hashlib.sha256(executable.read_bytes()).hexdigest()
        env = dict(os.environ)
        for name in ("HERDR_PREVIEW_MANIFEST_URL", "HERDR_UPDATE_MANIFEST_URL",
                     "HERDR_MANIFEST_URL", "HERDR_CHANNEL", "HERDR_INSTALLER_URL"):
            env.pop(name, None)
        env.update(HERDR_ENV="0", HERDR_USAGE_DISABLED="1",
                   HERDR_HOME=str(root / "home"), HERDR_CONFIG_PATH=str(root / "config.toml"),
                   XDG_CONFIG_HOME=str(root / "config"), XDG_STATE_HOME=str(root / "state"),
                   HERDR_SOCKET_PATH=str(root / "server.sock"),
                   HERDR_CLIENT_SOCKET_PATH=str(root / "client.sock"))
        if os.name == "nt":
            # curl remains available, while an unexpected installer cannot
            # launch Windows PowerShell or alter the user's PATH.
            env["PATH"] = str(Path(os.environ["SystemRoot"]) / "System32")

        def run(*arguments):
            result = subprocess.run([str(executable), *arguments], env=env, cwd=root,
                                    text=True, capture_output=True, timeout=60)
            return result.returncode, result.stdout + result.stderr

        assert run("--version") == (0, f"herdr {args.tag}\n")
        for configuration in ("", '[update]\nchannel = "stable"\n'):
            (root / "config.toml").write_text(configuration, encoding="utf-8")
            assert run("channel", "show") == (0, "preview\n")
            status, output = run("update")
            assert status == 0 and f"already up to date ({args.tag})" in output, output
            assert "installing " not in output, output
            print(output.strip())
        before = (root / "config.toml").read_bytes()
        status, output = run("channel", "set", "stable")
        assert status != 0 and "preview releases only" in output, output
        assert (root / "config.toml").read_bytes() == before
        foreign = dict(manifest, build_id="2026-10-14-fbd2ae23927f", base_version="0.9.4")
        foreign_path = root / "foreign.json"
        foreign_path.write_text(json.dumps(foreign), encoding="utf-8")
        env["HERDR_PREVIEW_MANIFEST_URL"] = foreign_path.as_uri()
        status, output = run("update")
        assert status != 0 and "Refusing to replace a fork build" in output, output
        assert hashlib.sha256(executable.read_bytes()).hexdigest() == digest
        env.pop("HERDR_PREVIEW_MANIFEST_URL")
        # Verify the published installers, rather than only the checkout.
        suffixes = ("ps1", "cmd") if os.name == "nt" else ("sh",)
        for suffix in suffixes:
            url = f"https://github.com/zubo334/herdr/releases/download/{args.tag}/install.{suffix}"
            with urllib.request.urlopen(url, timeout=60) as response:
                script = response.read()
            source = Path(__file__).resolve().parents[1] / "distribution" / f"install.{suffix}"
            assert script.replace(b"\r\n", b"\n") == source.read_bytes().replace(b"\r\n", b"\n")
            (root / f"install.{suffix}").write_bytes(script)
        env["HERDR_INSTALL_DIR"] = str(root / "installed")
        if os.name != "nt":
            result = subprocess.run(["/bin/sh", str(root / "install.sh")], env=env, cwd=root,
                                    text=True, capture_output=True, timeout=180)
            assert result.returncode == 0, result.stdout + result.stderr
            installed = root / "installed/herdr"
            assert hashlib.sha256(installed.read_bytes()).hexdigest() == digest
        else:
            # Exercise CMD's actual download/bootstrap without installing or
            # changing the user's PATH: invalid channel exits before setup.
            powershell_dir = Path(os.environ["SystemRoot"]) / "System32/WindowsPowerShell/v1.0"
            env["PATH"] += os.pathsep + str(powershell_dir)
            result = subprocess.run(
                ["cmd.exe", "/d", "/c", str(root / "install.cmd"), "-Channel", "invalid"],
                env=env, cwd=root, text=True, capture_output=True, timeout=120,
            )
            assert result.returncode != 0 and "Invalid Herdr channel" in result.stdout + result.stderr, result
    print(f"{args.target}: public fork defaults, legacy config, and foreign-feed rejection passed")


if __name__ == "__main__":
    main()
