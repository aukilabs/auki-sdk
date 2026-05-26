#!/usr/bin/env python3
from __future__ import annotations

import importlib.util
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("check-binding-api-coverage.py")


def load_checker():
    spec = importlib.util.spec_from_file_location("binding_api_coverage", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    assert spec and spec.loader
    spec.loader.exec_module(module)
    return module


class BindingApiCoverageTests(unittest.TestCase):
    def write_crate(
        self,
        root: Path,
        *,
        core: str,
        ffi: str,
        wasm: str,
    ) -> Path:
        crate = root / "sample"
        src = crate / "src"
        src.mkdir(parents=True)
        (crate / "bindings.toml").write_text("", encoding="utf-8")
        (src / "core.rs").write_text(core, encoding="utf-8")
        (src / "ffi.rs").write_text(ffi, encoding="utf-8")
        (src / "wasm.rs").write_text(wasm, encoding="utf-8")
        return crate

    def test_reports_missing_items_and_accepts_adapter_names(self):
        checker = load_checker()
        with tempfile.TemporaryDirectory() as tmp:
            crate = self.write_crate(
                Path(tmp),
                core="""
pub struct Wallet;

impl Wallet {
    pub fn new() -> Self { Self }
    pub fn seed(&self) -> Vec<u8> { vec![] }
}

pub fn canonicalize(value: &serde_json::Value) -> Vec<u8> { vec![] }
pub fn missing_function() {}
""",
                ffi="""
pub struct Wallet;

impl Wallet {
    pub fn new() -> Self { Self }
    pub fn seed(&self) -> Vec<u8> { vec![] }
}

pub fn canonicalize_json(json: String) -> Vec<u8> { vec![] }
""",
                wasm="""
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct Wallet;

#[wasm_bindgen]
impl Wallet {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Wallet { Wallet }

    pub fn seed(&self) -> Vec<u8> { vec![] }
}

#[wasm_bindgen(js_name = canonicalizeJson)]
pub fn canonicalize_json(json: String) -> Vec<u8> { vec![] }
""",
            )

            report = checker.analyze_crate(crate, "sample")

        self.assertEqual(
            checker.missing_names(report, "uniffi"),
            ["fn missing_function"],
        )
        self.assertEqual(
            checker.missing_names(report, "wasm"),
            ["fn missing_function"],
        )

    def test_code_adjacent_exclusions_suppress_only_named_target(self):
        checker = load_checker()
        with tempfile.TemporaryDirectory() as tmp:
            crate = self.write_crate(
                Path(tmp),
                core="""
// binding-exclude: wasm
// reason: browser storage adapter is intentionally different.
pub fn load_or_mint_seed(path: &std::path::Path) -> Vec<u8> { vec![] }
""",
                ffi="pub fn load_or_mint_seed(path: String) -> Vec<u8> { vec![] }",
                wasm="",
            )

            report = checker.analyze_crate(crate, "sample")

        self.assertEqual(checker.missing_names(report, "uniffi"), [])
        self.assertEqual(checker.missing_names(report, "wasm"), [])
        self.assertEqual(
            [item["display"] for item in report["targets"]["wasm"]["excluded"]],
            ["fn load_or_mint_seed"],
        )

    def test_exclusions_without_reason_are_reported(self):
        checker = load_checker()
        with tempfile.TemporaryDirectory() as tmp:
            crate = self.write_crate(
                Path(tmp),
                core="""
// binding-exclude: uniffi, wasm
pub trait GenericPayload {}
""",
                ffi="",
                wasm="",
            )

            report = checker.analyze_crate(crate, "sample")

        self.assertEqual(checker.missing_names(report, "uniffi"), [])
        self.assertEqual(checker.missing_names(report, "wasm"), [])
        self.assertEqual(len(report["exclusion_warnings"]), 1)
        self.assertIn("missing reason", report["exclusion_warnings"][0]["message"])


if __name__ == "__main__":
    unittest.main()
