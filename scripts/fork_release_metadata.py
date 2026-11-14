#!/usr/bin/env python3
"""Resolve one immutable identity shared by fork binaries and manifests."""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import re
import subprocess
import tomllib


def git(root: Path, *args: str) -> str:
    return subprocess.check_output(["git", "-C", str(root), *args], text=True).strip()


def metadata(root: Path, version: str, ref_name: str) -> dict[str, str]:
    if not version.strip():
        if not ref_name.startswith("fork-v"):
            raise ValueError("fork release requires a fork-v tag or an explicit version")
        version = ref_name.removeprefix("fork-v")
    version = version.strip().removeprefix("v")
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?", version):
        raise ValueError("invalid fork release version")
    tag = f"fork-v{version}"
    commit = git(root, "rev-parse", "HEAD")
    existing = subprocess.run(
        ["git", "-C", str(root), "rev-parse", "--verify", f"refs/tags/{tag}^{{commit}}"],
        text=True, capture_output=True,
    )
    if existing.returncode == 0 and existing.stdout.strip() != commit:
        raise ValueError(f"{tag} points at a different commit")

    def constant(path: str, name: str) -> str:
        match = re.search(rf"\b{name}\s*:\s*u32\s*=\s*([0-9]+)",
                          (root / path).read_text(encoding="utf-8"))
        if not match:
            raise ValueError(f"missing {name}")
        return match.group(1)

    committed_at = datetime.fromisoformat(git(root, "show", "-s", "--format=%cI", "HEAD"))
    return {
        "version": version,
        "tag": tag,
        "channel": "preview",
        "build_id": tag,
        "commit": commit,
        "base_version": tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))["package"]["version"],
        "built_at": committed_at.astimezone(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z"),
        "protocol": constant("src/protocol/wire.rs", "PROTOCOL_VERSION"),
        "endpoint_generation": constant("src/protocol/endpoint.rs", "ENDPOINT_PROTOCOL_GENERATION"),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--github-output", type=Path)
    parser.add_argument("--json-output", type=Path)
    args = parser.parse_args()
    try:
        values = metadata(Path.cwd(), os.environ.get("FORK_RELEASE_VERSION", ""),
                          os.environ.get("GITHUB_REF_NAME", ""))
    except (ValueError, subprocess.CalledProcessError) as error:
        parser.error(str(error))
    if args.json_output:
        args.json_output.write_text(json.dumps(values, indent=2) + "\n", encoding="utf-8")
    if args.github_output:
        with args.github_output.open("a", encoding="utf-8") as output:
            for key, value in values.items():
                output.write(f"{key}={value}\n")
    elif not args.json_output:
        print(json.dumps(values, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
