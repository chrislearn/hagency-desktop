#!/usr/bin/env python3
"""Reject unpublished sibling dependencies in the owner UI workspaces.

Two workspaces link the Client: Desktop and the owner-dashboard crate.
"""
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
CLIENT = "https://github.com/hagency-org/hagency-client.git"
MANIFESTS = ["Cargo.toml", "crates/owner-dashboard/Cargo.toml"]


def main():
    pins = set()
    checked = []
    for name in MANIFESTS:
        manifest = ROOT / name
        text = manifest.read_text()
        for package, body in re.findall(r'^(hagency(?:-core|-metering)?)\s*=\s*\{([^\n]+)\}', text, re.M):
            source = re.search(r'git\s*=\s*"([^"]+)"', body)
            revision = re.search(r'rev\s*=\s*"([0-9a-f]{40})"', body)
            if not source or source[1] != CLIENT or not revision or re.search(r'\bpath\s*=', body):
                raise SystemExit(f"{name}: {package} must use a fixed, published Hagency Client commit")
            pins.add(revision[1])
            checked.append(f"{name}:{package}")
    if len(checked) != 4 or len(pins) != 1:
        raise SystemExit("Desktop and owner-dashboard must share exactly one Client revision (including test dependencies)")
    pin = pins.pop()
    expected = f"git+{CLIENT}?rev={pin}#{pin}"
    for name in MANIFESTS:
        lock = (ROOT / name).with_name("Cargo.lock")
        packages = lock.read_text().split("[[package]]")[1:]
        native = [p for p in packages if re.search(r'^name = "hagency(?:-(?!owner-)[^"\n]+)?"$', p, re.M)]
        if not native or any(f'source = "{expected}"' not in p for p in native):
            raise SystemExit(f"{lock.relative_to(ROOT)}: Client packages do not resolve exclusively to {pin}")
    print(json.dumps({"clientRevision": pin, "checked": checked, "lockfiles": 2}))


if __name__ == "__main__":
    main()
