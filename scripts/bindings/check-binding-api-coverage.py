#!/usr/bin/env python3
"""Report binding coverage gaps from code, not hand-written inventories."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path
from typing import Any


TARGETS = ("uniffi", "wasm")
DECL_RE = re.compile(r"^\s*pub\s+(?P<kind>struct|enum|trait|type|const|static|fn)\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)")
METHOD_RE = re.compile(r"^\s*pub\s+(?:async\s+)?fn\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)")
IMPL_RE = re.compile(r"^\s*impl(?:<[^>{}]+>)?\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)")
PUB_USE_RE = re.compile(r"^\s*pub\s+use\s+(?P<body>.+);")
WASM_NAME_RE = re.compile(r"(?:js_name|js_class)\s*=\s*(?P<name>[A-Za-z_][A-Za-z0-9_]*)")
EXCLUDE_RE = re.compile(r"binding-exclude:\s*(?P<targets>.+)", re.IGNORECASE)
REASON_RE = re.compile(r"reason:\s*(?P<reason>.+)", re.IGNORECASE)


def repo_root() -> Path:
    return Path(__file__).resolve().parents[2]


def cargo_metadata(root: Path) -> dict[str, Any]:
    return json.loads(
        subprocess.check_output(
            ["cargo", "metadata", "--format-version", "1", "--no-deps"],
            cwd=root,
            text=True,
        )
    )


def binding_crates(root: Path) -> list[tuple[str, Path]]:
    crates: list[tuple[str, Path]] = []
    for package in sorted(cargo_metadata(root)["packages"], key=lambda item: item["name"]):
        crate = Path(package["manifest_path"]).parent
        if (crate / "bindings.toml").exists():
            crates.append((package["name"], crate))
    return crates


def normalize_name(name: str) -> str:
    return re.sub(r"[^a-z0-9]", "", name.lower())


def snake_to_camel(name: str) -> str:
    parts = name.split("_")
    return parts[0] + "".join(part[:1].upper() + part[1:] for part in parts[1:])


def target_names(raw: str) -> set[str]:
    names: set[str] = set()
    for part in re.split(r"[, ]+", raw.strip()):
        token = part.strip().lower()
        if not token:
            continue
        if token in {"all", "both"}:
            names.update(TARGETS)
        elif token in {"native", "python", "swift"}:
            names.add("uniffi")
        elif token in {"browser", "javascript", "js", "web"}:
            names.add("wasm")
        elif token in TARGETS:
            names.add(token)
    return names


def comment_block_before(lines: list[str], index: int) -> list[str]:
    comments: list[str] = []
    cursor = index - 1
    while cursor >= 0:
        stripped = lines[cursor].strip()
        if not stripped:
            cursor -= 1
            continue
        if stripped.startswith("#["):
            cursor -= 1
            continue
        if stripped.startswith("//"):
            comments.append(stripped.removeprefix("//").strip())
            cursor -= 1
            continue
        break
    comments.reverse()
    return comments


def exclusions_for(lines: list[str], index: int) -> tuple[set[str], str | None, list[str]]:
    comments = comment_block_before(lines, index)
    excluded: set[str] = set()
    reason: str | None = None
    warnings: list[str] = []
    for comment in comments:
        exclude = EXCLUDE_RE.search(comment)
        if exclude:
            excluded.update(target_names(exclude.group("targets")))
        reason_match = REASON_RE.search(comment)
        if reason_match and reason_match.group("reason").strip():
            reason = reason_match.group("reason").strip()
    if excluded and not reason:
        warnings.append("binding-exclude is missing reason")
    return excluded, reason, warnings


def api_item(kind: str, name: str, path: Path, line: int, *, owner: str | None = None, exclusions: set[str] | None = None, reason: str | None = None) -> dict[str, Any]:
    display = f"{kind} {owner + '::' if owner else ''}{name}"
    return {
        "kind": kind,
        "name": name,
        "owner": owner,
        "display": display,
        "path": str(path),
        "line": line,
        "exclusions": sorted(exclusions or []),
        "reason": reason,
    }


def parse_pub_use_names(body: str) -> list[str]:
    body = body.strip()
    if "{" in body and "}" in body:
        inside = body[body.index("{") + 1 : body.rindex("}")]
        names = []
        for raw in inside.split(","):
            name = raw.strip()
            if not name or name == "self":
                continue
            names.append(name.split(" as ")[-1].strip())
        return names
    tail = body.split(" as ")[-1].strip()
    return [tail.rsplit("::", 1)[-1]]


def brace_delta(line: str) -> int:
    return line.count("{") - line.count("}")


def parse_api_items(crate_dir: Path) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    source = crate_dir / "src" / "core.rs"
    if not source.exists():
        source = crate_dir / "src" / "lib.rs"
    lines = source.read_text(encoding="utf-8").splitlines()
    items: list[dict[str, Any]] = []
    warnings: list[dict[str, Any]] = []
    depth = 0
    index = 0

    while index < len(lines):
        line = lines[index]
        if depth == 0:
            decl = DECL_RE.match(line)
            pub_use = PUB_USE_RE.match(line)
            impl_match = IMPL_RE.match(line)
            if decl:
                excluded, reason, warning_messages = exclusions_for(lines, index)
                item = api_item(
                    decl.group("kind"),
                    decl.group("name"),
                    source,
                    index + 1,
                    exclusions=excluded,
                    reason=reason,
                )
                items.append(item)
                for message in warning_messages:
                    warnings.append({**item, "message": message})
            elif pub_use:
                excluded, reason, warning_messages = exclusions_for(lines, index)
                for name in parse_pub_use_names(pub_use.group("body")):
                    item = api_item("use", name, source, index + 1, exclusions=excluded, reason=reason)
                    items.append(item)
                    for message in warning_messages:
                        warnings.append({**item, "message": message})
            elif impl_match and " for " not in line:
                owner = impl_match.group("name")
                impl_depth = brace_delta(line)
                index += 1
                while index < len(lines) and impl_depth > 0:
                    method_line = lines[index]
                    method = METHOD_RE.match(method_line)
                    if method and impl_depth == 1:
                        excluded, reason, warning_messages = exclusions_for(lines, index)
                        item = api_item(
                            "method",
                            method.group("name"),
                            source,
                            index + 1,
                            owner=owner,
                            exclusions=excluded,
                            reason=reason,
                        )
                        items.append(item)
                        for message in warning_messages:
                            warnings.append({**item, "message": message})
                    impl_depth += brace_delta(method_line)
                    index += 1
                continue
        depth += brace_delta(line)
        index += 1

    return items, warnings


def wasm_export_names(lines: list[str], index: int) -> list[str]:
    names: list[str] = []
    for comment in comment_block_before(lines, index):
        match = WASM_NAME_RE.search(comment)
        if match:
            names.append(match.group("name"))
    return names


def parse_exports(path: Path, *, wasm: bool) -> dict[str, set[str]]:
    if not path.exists():
        return {"items": set(), "types": set(), "full": set()}
    lines = path.read_text(encoding="utf-8").splitlines()
    exports: dict[str, set[str]] = {"items": set(), "types": set(), "full": set()}
    depth = 0
    index = 0

    def add_item(name: str, *, owner: str | None = None, is_type: bool = False) -> None:
        variants = {name, snake_to_camel(name), normalize_name(name)}
        for variant in variants:
            exports["items"].add(normalize_name(variant))
            if owner:
                exports["full"].add(normalize_name(f"{owner}::{variant}"))
        if is_type:
            for variant in variants:
                exports["types"].add(normalize_name(variant))

    while index < len(lines):
        line = lines[index]
        if depth == 0:
            decl = DECL_RE.match(line)
            impl_match = IMPL_RE.match(line)
            if decl:
                is_type = decl.group("kind") in {"struct", "enum", "trait", "type"}
                add_item(decl.group("name"), is_type=is_type)
                if wasm:
                    for name in wasm_export_names(lines, index):
                        add_item(name, is_type=is_type)
            elif impl_match and " for " not in line:
                owner = impl_match.group("name")
                owner_names = {owner, *wasm_export_names(lines, index)}
                impl_depth = brace_delta(line)
                index += 1
                while index < len(lines) and impl_depth > 0:
                    method_line = lines[index]
                    method = METHOD_RE.match(method_line)
                    if method and impl_depth == 1:
                        method_names = {method.group("name")}
                        if wasm:
                            method_names.update(wasm_export_names(lines, index))
                        for owner_name in owner_names:
                            for method_name in method_names:
                                add_item(method_name, owner=owner_name)
                    impl_depth += brace_delta(method_line)
                    index += 1
                continue
        depth += brace_delta(line)
        index += 1
    return exports


def type_matches(item_type: str, export_type: str) -> bool:
    return item_type == export_type or export_type.endswith(item_type) or item_type.endswith(export_type)


def covered(item: dict[str, Any], exports: dict[str, set[str]]) -> bool:
    name = normalize_name(item["name"])
    owner = normalize_name(item["owner"] or "")
    full = normalize_name(f"{item['owner']}::{item['name']}") if item["owner"] else ""

    if full and full in exports["full"]:
        return True
    if item["kind"] == "method":
        if item["name"] == "new" and any(type_matches(owner, exported) for exported in exports["types"]):
            return True
        for exported_full in exports["full"]:
            if owner and name in exported_full and owner in exported_full:
                return True
        for exported in exports["items"]:
            if owner and name in exported and owner in exported:
                return True
        return False

    if name in exports["items"] or name in exports["types"]:
        return True
    for exported in exports["items"] | exports["types"]:
        if exported.startswith(name) or name.startswith(exported):
            return True
    return False


def analyze_crate(crate_dir: Path, crate_name: str) -> dict[str, Any]:
    items, exclusion_warnings = parse_api_items(crate_dir)
    exports = {
        "uniffi": parse_exports(crate_dir / "src" / "ffi.rs", wasm=False),
        "wasm": parse_exports(crate_dir / "src" / "wasm.rs", wasm=True),
    }
    targets: dict[str, Any] = {}
    for target in TARGETS:
        covered_items = []
        missing = []
        excluded = []
        for item in items:
            if target in item["exclusions"]:
                excluded.append(item)
            elif covered(item, exports[target]):
                covered_items.append(item)
            else:
                missing.append(item)
        targets[target] = {
            "covered": covered_items,
            "missing": missing,
            "excluded": excluded,
        }
    return {
        "crate": crate_name,
        "crate_dir": str(crate_dir),
        "api_items": items,
        "targets": targets,
        "exclusion_warnings": exclusion_warnings,
    }


def missing_names(report: dict[str, Any], target: str) -> list[str]:
    return [item["display"] for item in report["targets"][target]["missing"]]


def report_has_gaps(report: dict[str, Any]) -> bool:
    return bool(
        report["exclusion_warnings"]
        or report["targets"]["uniffi"]["missing"]
        or report["targets"]["wasm"]["missing"]
    )


def print_text_report(reports: list[dict[str, Any]]) -> None:
    print("Binding API coverage report")
    print("Source of truth: public API items in src/core.rs, falling back to src/lib.rs.")
    print("Exclusions: code-adjacent `binding-exclude:` comments with a `reason:` line.")
    print()
    for report in reports:
        print(f"{report['crate']}")
        for target in TARGETS:
            target_report = report["targets"][target]
            covered_count = len(target_report["covered"])
            missing = target_report["missing"]
            excluded = target_report["excluded"]
            status = "OK" if not missing else f"{len(missing)} gap(s)"
            print(
                f"  {target}: {status}; covered {covered_count}, excluded {len(excluded)}, total {len(report['api_items'])}"
            )
            for item in missing:
                print(f"    missing {item['display']} ({item['path']}:{item['line']})")
            for item in excluded:
                print(f"    excluded {item['display']}: {item.get('reason') or 'missing reason'}")
        if report["exclusion_warnings"]:
            print("  exclusion warnings:")
            for warning in report["exclusion_warnings"]:
                print(f"    {warning['display']} ({warning['path']}:{warning['line']}): {warning['message']}")
        print()


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("crates", nargs="*", help="Optional crate package names to check")
    parser.add_argument("--json", action="store_true", help="Print machine-readable JSON")
    parser.add_argument("--fail-on-gaps", action="store_true", help="Exit 1 when any gap or invalid exclusion is reported")
    args = parser.parse_args(argv)

    root = repo_root()
    selected = set(args.crates)
    reports = [
        analyze_crate(crate_dir, crate_name)
        for crate_name, crate_dir in binding_crates(root)
        if not selected or crate_name in selected
    ]
    unknown = selected - {report["crate"] for report in reports}
    if unknown:
        print(f"unknown or non-binding-enabled crate(s): {', '.join(sorted(unknown))}", file=sys.stderr)
        return 2

    if args.json:
        print(json.dumps(reports, indent=2, sort_keys=True))
    else:
        print_text_report(reports)

    if args.fail_on_gaps and any(report_has_gaps(report) for report in reports):
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
