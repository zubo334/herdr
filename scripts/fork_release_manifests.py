#!/usr/bin/env python3
"""Generate the native preview feed and the flat Unix installer feed together."""
from __future__ import annotations

import argparse
from copy import deepcopy
import hashlib
import json
from pathlib import Path

ASSETS = {
    "linux-x86_64": "herdr-linux-x86_64",
    "linux-aarch64": "herdr-linux-aarch64",
    "macos-x86_64": "herdr-macos-x86_64",
    "macos-aarch64": "herdr-macos-aarch64",
    "windows-x86_64": "herdr-windows-x86_64.zip",
}
BUILD_FIELDS = ("base_version", "commit", "built_at", "protocol", "endpoint_generation", "assets")


def manifests(metadata: dict, repository: str, root: Path, previous: dict) -> tuple[dict, dict]:
    tag = metadata["tag"]
    if metadata["build_id"] != tag or not tag.startswith("fork-v"):
        raise ValueError("fork build identity must match its release tag")
    assets = {}
    for target, filename in ASSETS.items():
        payload = root / filename / filename
        asset = {
            "url": f"https://github.com/{repository}/releases/download/{tag}/{filename}",
            "sha256": hashlib.sha256(payload.read_bytes()).hexdigest(),
        }
        if target.startswith("windows-"):
            asset["format"] = "zip"
        assets[target] = asset
    preview = {
        "schema_version": 1, "channel": "preview",
        "base_version": metadata["base_version"], "build_id": tag,
        "commit": metadata["commit"], "built_at": metadata["built_at"],
        "protocol": int(metadata["protocol"]),
        "endpoint_generation": int(metadata["endpoint_generation"]),
        "notes": f"Fork release {tag}", "assets": assets,
    }
    if previous.get("channel") != "preview" or not previous.get("build_id", "").startswith("fork-v"):
        raise ValueError("previous manifest is not a fork preview")
    builds = deepcopy(previous.get("builds", {}))
    old_tag = previous["build_id"]
    if old_tag == tag and previous["commit"] != metadata["commit"]:
        raise ValueError("published build identity points at a different commit")
    builds[old_tag] = {key: previous[key] for key in BUILD_FIELDS if key in previous}
    builds[tag] = {key: deepcopy(preview[key]) for key in BUILD_FIELDS}
    preview["builds"] = builds
    # latest.json is an installer feed, explicitly marked preview. It is not
    # a stable channel: this fork publishes preview binaries only.
    installer = {
        key: deepcopy(value)
        for key, value in preview.items() if key not in ("assets", "builds")
    }
    installer["version"] = tag
    installer["assets"] = {target: value["url"] for target, value in assets.items()}
    installer["sha256"] = {target: value["sha256"] for target, value in assets.items()}
    return preview, installer


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--metadata", type=Path, required=True)
    parser.add_argument("--repository", required=True)
    parser.add_argument("--previous", type=Path, required=True)
    args = parser.parse_args()
    preview, installer = manifests(
        json.loads(args.metadata.read_text(encoding="utf-8")), args.repository,
        Path.cwd(), json.loads(args.previous.read_text(encoding="utf-8")),
    )
    for filename, value in [("preview.json", preview), ("latest.json", installer)]:
        Path(filename).write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
