#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

metadata_json="$(python3 scripts/bindings/generate_bindings.py metadata auki-uniffi-test)"
javascript_plan_json="$(python3 scripts/bindings/generate_bindings.py plan javascript auki-uniffi-test)"
python_plan_json="$(python3 scripts/bindings/generate_bindings.py plan python auki-uniffi-test)"
swift_plan_json="$(python3 scripts/bindings/generate_bindings.py plan swift auki-uniffi-test)"
swift_xcframework_plan_json="$(python3 scripts/bindings/generate_bindings.py plan swift-xcframework auki-uniffi-test)"

python3 - "$metadata_json" "$javascript_plan_json" "$python_plan_json" "$swift_plan_json" "$swift_xcframework_plan_json" <<'PY'
import json
import sys

metadata = json.loads(sys.argv[1])
javascript_plan = json.loads(sys.argv[2])
python_plan = json.loads(sys.argv[3])
swift_plan = json.loads(sys.argv[4])
swift_xcframework_plan = json.loads(sys.argv[5])

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
PY

echo "binding generator contract ok"
