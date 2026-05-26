#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

metadata_json="$(python3 scripts/bindings/generate_bindings.py metadata auki-uniffi-test)"
inventory_json="$(python3 scripts/bindings/generate_bindings.py list)"
javascript_plan_json="$(python3 scripts/bindings/generate_bindings.py plan javascript auki-uniffi-test)"
python_plan_json="$(python3 scripts/bindings/generate_bindings.py plan python auki-uniffi-test)"
swift_plan_json="$(python3 scripts/bindings/generate_bindings.py plan swift auki-uniffi-test)"
swift_xcframework_plan_json="$(python3 scripts/bindings/generate_bindings.py plan swift-xcframework auki-uniffi-test)"

python3 - "$metadata_json" "$inventory_json" "$javascript_plan_json" "$python_plan_json" "$swift_plan_json" "$swift_xcframework_plan_json" <<'PY'
import json
import subprocess
import sys

metadata = json.loads(sys.argv[1])
inventory = json.loads(sys.argv[2])
javascript_plan = json.loads(sys.argv[3])
python_plan = json.loads(sys.argv[4])
swift_plan = json.loads(sys.argv[5])
swift_xcframework_plan = json.loads(sys.argv[6])

assert metadata["package_name"] == "auki-uniffi-test"
assert metadata["version"] == "0.0.0"
assert metadata["lib_name"] == "auki_uniffi_test"
assert metadata["repository"] == "https://github.com/aukilabs/auki-sdk"
assert metadata["authors"] == ["Auki Labs Limited"]

assert javascript_plan["language"] == "javascript"
assert javascript_plan["generator"] == "wasm_pack"
assert javascript_plan["crate_assets_dir"] == "crates/auki-uniffi-test/bindings/javascript"
assert javascript_plan["output_dir"] == "bindings/javascript/auki-uniffi-test"
assert javascript_plan["template_files"] == [
    "package.json.tmpl",
    "README.md.tmpl",
]
assert javascript_plan["smoke"] == "crates/auki-uniffi-test/bindings/javascript/smoke.mjs"

assert python_plan["language"] == "python"
assert python_plan["generator"] == "uniffi"
assert python_plan["crate_assets_dir"] == "crates/auki-uniffi-test/bindings/python"
assert python_plan["output_dir"] == "bindings/python/auki-uniffi-test"
assert python_plan["template_files"] == [
    "pyproject.toml.tmpl",
    "setup.py.tmpl",
    "README.md.tmpl",
]

assert swift_plan["language"] == "swift"
assert swift_plan["generator"] == "uniffi"
assert swift_plan["crate_assets_dir"] == "crates/auki-uniffi-test/bindings/swift"
assert swift_plan["output_dir"] == "bindings/swift/auki-uniffi-test"
assert swift_plan["template_files"] == ["Package.swift.tmpl"]

assert swift_xcframework_plan["language"] == "swift-xcframework"
assert swift_xcframework_plan["binding_language"] == "swift"
assert swift_xcframework_plan["generator"] == "uniffi"
assert swift_xcframework_plan["output_dir"] == "bindings/swift/auki-uniffi-test"

names = {crate["package_name"] for crate in inventory}
assert "auki-uniffi-test" in names
assert len(inventory) >= 2

for crate in inventory:
    assert crate["enabled_languages"], f"{crate['package_name']} has no enabled bindings"
    for language in crate["enabled_languages"]:
        binding = crate["bindings"][language]
        assert not binding["missing_templates"], (
            f"{crate['package_name']} {language} missing templates: "
            f"{binding['missing_templates']}"
        )
        assert not binding["missing_smoke"], f"{crate['package_name']} {language} missing smoke file"
        assert not binding["missing_features"], (
            f"{crate['package_name']} {language} missing features: "
            f"{binding['missing_features']}"
        )
        planned = json.loads(
            subprocess.check_output(
                [
                    "python3",
                    "scripts/bindings/generate_bindings.py",
                    "plan",
                    language,
                    crate["package_name"],
                ],
                text=True,
            )
        )
        assert planned["crate_assets_dir"] == binding["crate_assets_dir"]
        assert planned["output_dir"] == binding["output_dir"]
        assert planned["template_files"] == binding["template_files"]
    if "swift" in crate["enabled_languages"]:
        swift_xcframework = json.loads(
            subprocess.check_output(
                [
                    "python3",
                    "scripts/bindings/generate_bindings.py",
                    "plan",
                    "swift-xcframework",
                    crate["package_name"],
                ],
                text=True,
            )
        )
        assert swift_xcframework["binding_language"] == "swift"
        assert swift_xcframework["output_dir"] == crate["bindings"]["swift"]["output_dir"]
PY

echo "binding generator contract ok"
