#!/usr/bin/env python3
"""Patch UniFFI's generated Python loader to use packaged native libraries."""

from __future__ import annotations

import argparse
from pathlib import Path
import textwrap


def replacement(lib_name: str) -> str:
    env_prefix = lib_name.upper()
    return textwrap.dedent(
        f'''
        def _uniffi_normalized_machine():
            machine = platform.machine().lower()
            if machine in ("amd64", "x86_64"):
                return "x86_64"
            if machine in ("arm64", "aarch64"):
                return "aarch64"
            return machine

        def _uniffi_native_target():
            override = os.environ.get("{env_prefix}_NATIVE_TARGET")
            if override:
                return override

            machine = _uniffi_normalized_machine()
            if sys.platform == "darwin":
                if machine in ("aarch64", "x86_64"):
                    return f"{{machine}}-apple-darwin"
            elif sys.platform.startswith("linux"):
                if machine in ("aarch64", "x86_64"):
                    return f"{{machine}}-unknown-linux-gnu"
            elif sys.platform.startswith("win"):
                if machine == "x86_64":
                    return "x86_64-pc-windows-msvc"

            raise RuntimeError(
                f"unsupported platform for {lib_name}: {{sys.platform}}/{{platform.machine()}}"
            )

        def _uniffi_library_filename():
            if sys.platform == "darwin":
                return "lib{lib_name}.dylib"
            if sys.platform.startswith("win"):
                return "{lib_name}.dll"
            return "lib{lib_name}.so"

        def _uniffi_load_indirect():
            """
            Load the native library bundled with this Python package.

            Set {env_prefix}_LIBRARY_PATH to force a specific dynamic library,
            or {env_prefix}_NATIVE_TARGET to force a packaged Rust target
            directory under native/.
            """
            override = os.environ.get("{env_prefix}_LIBRARY_PATH")
            if override:
                return ctypes.cdll.LoadLibrary(override)

            target = _uniffi_native_target()
            path = os.path.join(
                os.path.dirname(__file__),
                "native",
                target,
                _uniffi_library_filename(),
            )
            if not os.path.exists(path):
                raise RuntimeError(
                    f"missing native library for {lib_name} target {{target}}: {{path}}"
                )
            return ctypes.cdll.LoadLibrary(path)
        '''
    ).lstrip()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("generated_file", type=Path)
    parser.add_argument("lib_name")
    args = parser.parse_args()

    text = args.generated_file.read_text()
    start_marker = "def _uniffi_load_indirect():\n"
    end_marker = "def _uniffi_check_contract_api_version(lib):\n"
    start = text.index(start_marker)
    end = text.index(end_marker, start)
    patched = text[:start] + replacement(args.lib_name) + "\n" + text[end:]
    args.generated_file.write_text(patched)


if __name__ == "__main__":
    main()
