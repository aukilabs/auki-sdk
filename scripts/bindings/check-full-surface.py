#!/usr/bin/env python3
"""Verify full binding surface inventories have matching Rust test markers."""

from __future__ import annotations

import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
CRATES = ("auki-network", "auki-domain")
MARKER_RE = re.compile(r"binding-surface:\s*(?P<name>.+)", re.IGNORECASE)


def normalize(text: str) -> str:
    cleaned = text.strip()
    cleaned = cleaned.strip(".")
    cleaned = cleaned.replace("`", "")
    cleaned = cleaned.replace("JavaScript", "javascript")
    cleaned = cleaned.replace("DTO", "dto")
    return re.sub(r"\s+", " ", cleaned).lower()


def required_surfaces(crate: str) -> list[str]:
    path = ROOT / "crates" / crate / "bindings" / "surface.md"
    if not path.exists():
        raise FileNotFoundError(f"missing binding surface inventory: {path}")

    required: list[str] = []
    current_scope: str | None = None

    for raw_line in path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if line == "## Native UniFFI Required":
            current_scope = "native"
            continue
        if line == "## Browser JavaScript Required":
            current_scope = "browser"
            continue
        if line.startswith("## "):
            current_scope = None
            continue
        if current_scope and line.startswith("- "):
            required.append(normalize(f"{current_scope} {line[2:]}"))

    if not required:
        raise ValueError(f"no required binding surface items found in {path}")
    return required


def marked_surfaces(crate: str) -> set[str]:
    test_dir = ROOT / "crates" / crate / "tests"
    markers: set[str] = set()

    for test_file in sorted(test_dir.glob("*.rs")):
        for match in MARKER_RE.finditer(test_file.read_text(encoding="utf-8")):
            markers.add(normalize(match.group("name")))

    return markers


def check_crate(crate: str) -> list[str]:
    required = required_surfaces(crate)
    markers = marked_surfaces(crate)
    return [item for item in required if item not in markers]


def main() -> int:
    failed = False
    for crate in CRATES:
        missing = check_crate(crate)
        if missing:
            failed = True
            print(f"{crate} is missing binding surface test markers:", file=sys.stderr)
            for item in missing:
                print(f"  - {item}", file=sys.stderr)
            continue
        print(f"full binding surface markers present for {crate}")
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
