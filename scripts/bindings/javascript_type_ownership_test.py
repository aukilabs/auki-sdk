#!/usr/bin/env python3
from __future__ import annotations

import re
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]


def exported_declarations(path: Path) -> set[str]:
    text = path.read_text(encoding="utf-8")
    return set(re.findall(r"^export\s+(?:declare\s+)?(?:function|class)\s+([A-Za-z0-9_]+)\b", text, re.MULTILINE))


class JavascriptTypeOwnershipTests(unittest.TestCase):
    def assert_wrapper_does_not_redeclare_wasm_exports(self, package_name: str, module_name: str) -> None:
        wrapper_template = ROOT / "crates" / package_name / "bindings/javascript/index.d.ts.tmpl"
        generated_types = ROOT / "bindings/javascript" / package_name / f"{module_name}.d.ts"

        wasm_exports = exported_declarations(generated_types)
        wrapper_exports = exported_declarations(wrapper_template)
        duplicates = sorted(wasm_exports & wrapper_exports)

        self.assertFalse(
            duplicates,
            f"{wrapper_template.relative_to(ROOT)} redeclares wasm-pack exports: {duplicates}",
        )
        self.assertIn(
            'export * from "./$generated_js_file";',
            wrapper_template.read_text(encoding="utf-8"),
        )

    def test_auki_network_wrapper_reexports_wasm_pack_declarations(self):
        self.assert_wrapper_does_not_redeclare_wasm_exports("auki-network", "auki_network")

    def test_auki_domain_wrapper_reexports_wasm_pack_declarations(self):
        self.assert_wrapper_does_not_redeclare_wasm_exports("auki-domain", "auki_domain")


if __name__ == "__main__":
    unittest.main()
