#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import platform
import shutil
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path
from string import Template


LANGUAGES = {"python", "swift", "swift-xcframework", "javascript"}


class BindingError(RuntimeError):
    pass


def repo_root() -> Path:
    return Path(__file__).resolve().parents[2]


def run(cmd: list[str], *, cwd: Path | None = None, env: dict[str, str] | None = None) -> None:
    subprocess.run(cmd, cwd=cwd, env=env, check=True)


def output(cmd: list[str], *, cwd: Path | None = None) -> str:
    return subprocess.check_output(cmd, cwd=cwd, text=True)


def cargo_metadata(root: Path) -> dict:
    return json.loads(output(["cargo", "metadata", "--format-version", "1", "--no-deps"], cwd=root))


def cargo_package(root: Path, package_name: str) -> dict:
    metadata = cargo_metadata(root)
    for package in metadata["packages"]:
        if package["name"] == package_name:
            return package
    raise BindingError(f"crate not found in cargo metadata: {package_name}")


def lib_target(package: dict) -> dict:
    for target in package["targets"]:
        if "lib" in target["kind"] or Path(target["src_path"]).name == "lib.rs":
            return target
    raise BindingError(f"crate has no library target: {package['name']}")


def crate_dir(package: dict) -> Path:
    return Path(package["manifest_path"]).parent


def load_bindings_toml(crate_path: Path) -> dict:
    config_path = crate_path / "bindings.toml"
    if not config_path.exists():
        raise BindingError(f"missing crate binding config: {config_path}")
    with config_path.open("rb") as fh:
        return tomllib.load(fh)


def host_target(root: Path) -> str:
    for line in output(["rustc", "-vV"], cwd=root).splitlines():
        if line.startswith("host: "):
            return line.removeprefix("host: ")
    raise BindingError("could not determine rustc host target")


def dynamic_lib_ext() -> str:
    system = platform.system()
    if system == "Darwin":
        return "dylib"
    if system == "Linux":
        return "so"
    if system.startswith(("MINGW", "MSYS", "CYGWIN")) or system == "Windows":
        return "dll"
    raise BindingError(f"unsupported host OS: {system}")


def dynamic_lib_file(lib_name: str, target: str | None = None) -> str:
    if target and "-pc-windows-" in target:
        return f"{lib_name}.dll"
    ext = dynamic_lib_ext() if target is None else ("dylib" if target.endswith("-apple-darwin") else "so")
    return f"lib{lib_name}.{ext}"


def static_lib_file(lib_name: str) -> str:
    return f"lib{lib_name}.a"


def package_metadata(root: Path, package_name: str) -> dict:
    package = cargo_package(root, package_name)
    target = lib_target(package)
    authors = package.get("authors") or []
    return {
        "package_name": package["name"],
        "version": package["version"],
        "description": package.get("description") or "",
        "license": package.get("license") or "",
        "repository": package.get("repository") or "",
        "authors": authors,
        "authors_toml": ", ".join(f'{{ name = "{author}" }}' for author in authors),
        "manifest_path": rel(root, Path(package["manifest_path"])),
        "crate_dir": rel(root, crate_dir(package)),
        "lib_name": target["name"],
        "features": sorted(package.get("features", {}).keys()),
    }


def rel(root: Path, path: Path) -> str:
    return path.resolve().relative_to(root.resolve()).as_posix()


def binding_section(config: dict, language: str) -> tuple[str, dict]:
    requested = "swift" if language == "swift-xcframework" else language
    section = config.get("bindings", {}).get(requested)
    if not section or not section.get("enabled", False):
        raise BindingError(f"binding is not enabled for language: {requested}")
    return requested, section


def plan(root: Path, package_name: str, language: str) -> dict:
    if language not in LANGUAGES:
        raise BindingError(f"unsupported binding language: {language}")

    package = cargo_package(root, package_name)
    metadata = package_metadata(root, package_name)
    crate_path = crate_dir(package)
    config = load_bindings_toml(crate_path)
    binding_language, section = binding_section(config, language)
    generator = section["generator"]
    generator_config = config.get("generators", {}).get(generator, {})
    crate_assets_dir = crate_path / section.get("template_dir", f"bindings/{binding_language}")
    output_dir = root / section.get("output_dir", f"bindings/{binding_language}/{metadata['package_name']}")

    template_files = section.get("templates")
    if template_files is None:
        template_files = sorted(path.name for path in crate_assets_dir.glob("*.tmpl"))
    smoke = section.get("smoke")
    smoke_path = crate_path / smoke if smoke else None

    return {
        "language": language,
        "binding_language": binding_language,
        "generator": generator,
        "generator_config": generator_config,
        "crate_assets_dir": rel(root, crate_assets_dir),
        "output_dir": rel(root, output_dir),
        "template_files": template_files,
        "smoke": rel(root, smoke_path) if smoke_path else None,
        "module_name": section.get("module", metadata["lib_name"]),
        "metadata": metadata,
    }


def render_text(text: str, context: dict[str, object]) -> str:
    flat = flatten_context(context)
    return Template(text).safe_substitute(flat)


def flatten_context(context: dict[str, object], prefix: str = "") -> dict[str, str]:
    values: dict[str, str] = {}
    for key, value in context.items():
        name = f"{prefix}{key}"
        if isinstance(value, dict):
            values.update(flatten_context(value, f"{name}_"))
        elif isinstance(value, list):
            values[name] = ", ".join(str(item) for item in value)
            values[f"{name}_json"] = json.dumps(value)
        else:
            values[name] = str(value)
    return values


def template_context(binding_plan: dict) -> dict:
    metadata = binding_plan["metadata"]
    module_name = binding_plan["module_name"]
    return {
        "crate": metadata,
        "binding": {
            "language": binding_plan["binding_language"],
            "output_dir": binding_plan["output_dir"],
            "module_name": module_name,
        },
        "generated": {
            "js_file": f"{module_name}.js",
            "types_file": f"{module_name}.d.ts",
            "wasm_file": f"{module_name}_bg.wasm",
            "wasm_types_file": f"{module_name}_bg.wasm.d.ts",
        },
    }


def render_templates(root: Path, binding_plan: dict, dest: Path) -> None:
    assets = root / binding_plan["crate_assets_dir"]
    context = template_context(binding_plan)
    template_names = binding_section(load_bindings_toml(root / binding_plan["metadata"]["crate_dir"]), binding_plan["binding_language"])[1].get("templates")
    templates = [assets / name for name in template_names] if template_names else sorted(assets.glob("*.tmpl"))
    for template in templates:
        target = dest / template.name.removesuffix(".tmpl")
        target.write_text(render_text(template.read_text(), context), encoding="utf-8")


def render_asset(root: Path, source_rel: str, dest: Path, binding_plan: dict) -> None:
    source = root / source_rel
    context = template_context(binding_plan)
    dest.write_text(render_text(source.read_text(), context), encoding="utf-8")


def validate_features(metadata: dict, features: list[str]) -> None:
    missing = [feature for feature in features if feature not in metadata["features"]]
    if missing:
        raise BindingError(f"{metadata['package_name']} is missing binding generator features: {', '.join(missing)}")


def uniffi_generate(root: Path, package_name: str, language: str, out_dir: Path, library: Path, release: bool = False) -> None:
    binding_plan = plan(root, package_name, language)
    metadata = binding_plan["metadata"]
    generator_config = binding_plan["generator_config"]
    features = generator_config.get("features", ["cli"])
    validate_features(metadata, features)
    bindgen_bin = generator_config.get("bindgen_bin", "uniffi-bindgen")

    cmd = ["cargo", "run", "-p", package_name]
    if release:
        cmd.append("--release")
    cmd.extend(["--features", ",".join(features), "--bin", bindgen_bin, "--", "generate"])
    cmd.extend(["--library", str(library), "--language", binding_plan["binding_language"], "--out-dir", str(out_dir)])
    run(cmd, cwd=root)


def generate_python(root: Path, package_name: str) -> None:
    binding_plan = plan(root, package_name, "python")
    metadata = binding_plan["metadata"]
    lib_name = metadata["lib_name"]
    package_dir = root / binding_plan["output_dir"]
    module_dir = package_dir / binding_plan["module_name"]
    target = host_target(root)
    lib_file = dynamic_lib_file(lib_name)
    library = root / "target" / "debug" / lib_file

    run(["cargo", "build", "-p", package_name], cwd=root)
    if not library.exists():
        raise BindingError(f"expected UniFFI library not found: {library}")

    with tempfile.TemporaryDirectory(prefix=f"{package_name}.python-bindings.") as generated:
        generated_dir = Path(generated)
        uniffi_generate(root, package_name, "python", generated_dir, library)
        generated_py = generated_dir / f"{lib_name}.py"
        run(["python3", "scripts/patch-uniffi-python-loader.py", str(generated_py), lib_name], cwd=root)

        (module_dir / "native" / target).mkdir(parents=True, exist_ok=True)
        shutil.copy2(generated_py, module_dir / "__init__.py")
        shutil.copy2(library, module_dir / "native" / target / lib_file)

    render_templates(root, binding_plan, package_dir)
    print(f"Generated Python package in {rel(root, package_dir)}")
    print(f"Included host library for {target}")
    run(["python3", "scripts/bindings/generate_bindings.py", "python-native-libs", package_name], cwd=root)


def python_native_targets(root: Path, explicit: list[str]) -> list[str]:
    if explicit:
        return explicit
    env_targets = os.environ.get("AUKI_PYTHON_NATIVE_TARGETS")
    if env_targets:
        return env_targets.split()
    host = host_target(root)
    if platform.system() == "Darwin":
        return [host, "aarch64-apple-darwin", "x86_64-apple-darwin", "x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"]
    return [host, "x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"]


def build_python_native_libraries(root: Path, package_name: str, targets: list[str]) -> None:
    binding_plan = plan(root, package_name, "python")
    metadata = binding_plan["metadata"]
    lib_name = metadata["lib_name"]
    module_dir = root / binding_plan["output_dir"] / binding_plan["module_name"]

    if not (module_dir / "__init__.py").exists():
        raise BindingError(f"missing generated Python package: {module_dir / '__init__.py'}")

    seen: set[str] = set()
    for target in python_native_targets(root, targets):
        if target in seen:
            continue
        seen.add(target)

        if "-linux-" in target:
            builder = os.environ.get("CROSS", "cross")
            lib_file = f"lib{lib_name}.so"
        elif target.endswith("-apple-darwin"):
            builder = "cargo"
            lib_file = f"lib{lib_name}.dylib"
        elif "-pc-windows-" in target:
            builder = os.environ.get("CROSS", "cross")
            lib_file = f"{lib_name}.dll"
        else:
            raise BindingError(f"unsupported Python native-library target: {target}")

        if shutil.which(builder) is None:
            raise BindingError(f"required build tool not found for {target}: {builder}")

        env = os.environ.copy()
        if builder == os.environ.get("CROSS", "cross") and platform.system() == "Darwin" and platform.machine() == "arm64":
            env.setdefault("CROSS_CONTAINER_OPTS", "--platform linux/amd64")

        run([builder, "build", "--release", "-p", package_name, "--target", target], cwd=root, env=env)
        library = root / "target" / target / "release" / lib_file
        if not library.exists():
            raise BindingError(f"expected UniFFI library not found: {library}")

        target_dir = module_dir / "native" / target
        target_dir.mkdir(parents=True, exist_ok=True)
        shutil.copy2(library, target_dir / lib_file)
        print(f"Copied {rel(root, library)} to {rel(root, target_dir)}/")


def generate_javascript(root: Path, package_name: str) -> None:
    if shutil.which("wasm-pack") is None:
        raise BindingError("wasm-pack is required; run just install-toolchain")

    binding_plan = plan(root, package_name, "javascript")
    metadata = binding_plan["metadata"]
    generator_config = binding_plan["generator_config"]
    features = generator_config.get("features", ["wasm"])
    validate_features(metadata, features)

    crate_path = root / metadata["crate_dir"]
    package_dir = root / binding_plan["output_dir"]
    package_parent = package_dir.parent
    package_parent.mkdir(parents=True, exist_ok=True)

    tmp_dir = Path(tempfile.mkdtemp(prefix=f".{package_name}.tmp.", dir=package_parent))
    backup_dir: Path | None = None
    swapped = False
    try:
        run(
            [
                "wasm-pack",
                "build",
                ".",
                "--target",
                generator_config.get("target", "web"),
                "--out-dir",
                str(tmp_dir),
                "--no-default-features",
                "--features",
                ",".join(features),
            ],
            cwd=crate_path,
        )

        gitignore = tmp_dir / ".gitignore"
        if gitignore.exists():
            gitignore.unlink()

        render_templates(root, binding_plan, tmp_dir)
        if binding_plan["smoke"]:
            render_asset(root, binding_plan["smoke"], tmp_dir / Path(binding_plan["smoke"]).name, binding_plan)

        validate_package_json_files(tmp_dir / "package.json", tmp_dir)

        if package_dir.exists():
            backup_dir = Path(tempfile.mkdtemp(prefix=f".{package_name}.old.", dir=package_parent))
            backup_dir.rmdir()
            package_dir.rename(backup_dir)
        tmp_dir.rename(package_dir)
        swapped = True
        print(f"Generated JavaScript bindings in {rel(root, package_dir)}")
        run(["node", str(package_dir / "smoke.mjs")], cwd=root)
    except Exception:
        if swapped and backup_dir and backup_dir.exists():
            if package_dir.exists():
                shutil.rmtree(package_dir)
            backup_dir.rename(package_dir)
        raise
    finally:
        if tmp_dir.exists():
            shutil.rmtree(tmp_dir)
        if backup_dir and backup_dir.exists():
            shutil.rmtree(backup_dir)


def validate_package_json_files(package_json: Path, package_dir: Path) -> None:
    data = json.loads(package_json.read_text(encoding="utf-8"))
    missing = [name for name in data.get("files", []) if not (package_dir / name).exists()]
    if missing:
        raise BindingError(f"package.json references missing files: {', '.join(missing)}")


def generate_swift(root: Path, package_name: str) -> None:
    binding_plan = plan(root, package_name, "swift")
    metadata = binding_plan["metadata"]
    lib_name = metadata["lib_name"]
    package_dir = root / binding_plan["output_dir"]
    generated_dir = package_dir / "generated"
    lib_file = dynamic_lib_file(lib_name)
    library = root / "target" / "debug" / lib_file

    package_dir.mkdir(parents=True, exist_ok=True)
    render_templates(root, binding_plan, package_dir)
    run(["cargo", "build", "-p", package_name], cwd=root)
    if not library.exists():
        raise BindingError(f"expected UniFFI library not found: {library}")

    generated_dir.mkdir(parents=True, exist_ok=True)
    for name in [
        f"{lib_name}.swift",
        f"{lib_name}FFI.h",
        f"{lib_name}FFI.modulemap",
        f"lib{lib_name}.{dynamic_lib_ext()}",
    ]:
        for path in [package_dir / name, generated_dir / name]:
            if path.exists():
                path.unlink()

    uniffi_generate(root, package_name, "swift", generated_dir, library)
    shutil.copy2(library, generated_dir / lib_file)
    print(f"Generated Swift package sources in {rel(root, generated_dir)}")


def generate_swift_xcframework(root: Path, package_name: str) -> None:
    if platform.system() != "Darwin":
        raise BindingError("build-swift-xcframework requires macOS because it uses xcodebuild and lipo")

    binding_plan = plan(root, package_name, "swift-xcframework")
    metadata = binding_plan["metadata"]
    lib_name = metadata["lib_name"]
    package_dir = root / binding_plan["output_dir"]
    generated_dir = package_dir / "generated"
    headers_dir = generated_dir / "headers"

    package_dir.mkdir(parents=True, exist_ok=True)
    render_templates(root, binding_plan, package_dir)
    if generated_dir.exists():
        shutil.rmtree(generated_dir)
    headers_dir.mkdir(parents=True, exist_ok=True)

    targets = [
        "aarch64-apple-ios",
        "aarch64-apple-ios-sim",
        "x86_64-apple-ios",
        "aarch64-apple-darwin",
        "x86_64-apple-darwin",
    ]
    for target in targets:
        run(["cargo", "build", "--release", "-p", package_name, "--target", target], cwd=root)

    device_lib = root / "target/aarch64-apple-ios/release" / static_lib_file(lib_name)
    sim_fat = generated_dir / f"lib{lib_name}-sim.a"
    macos_fat = generated_dir / f"lib{lib_name}-macos.a"
    run(
        [
            "lipo",
            "-create",
            str(root / "target/aarch64-apple-ios-sim/release" / static_lib_file(lib_name)),
            str(root / "target/x86_64-apple-ios/release" / static_lib_file(lib_name)),
            "-output",
            str(sim_fat),
        ],
        cwd=root,
    )
    run(
        [
            "lipo",
            "-create",
            str(root / "target/aarch64-apple-darwin/release" / static_lib_file(lib_name)),
            str(root / "target/x86_64-apple-darwin/release" / static_lib_file(lib_name)),
            "-output",
            str(macos_fat),
        ],
        cwd=root,
    )

    uniffi_generate(root, package_name, "swift", headers_dir, device_lib, release=True)
    swift_file = headers_dir / f"{lib_name}.swift"
    if swift_file.exists():
        swift_file.rename(generated_dir / swift_file.name)
    modulemap = headers_dir / f"{lib_name}FFI.modulemap"
    if modulemap.exists():
        modulemap.rename(headers_dir / "module.modulemap")

    run(
        [
            "xcodebuild",
            "-create-xcframework",
            "-library",
            str(device_lib),
            "-headers",
            str(headers_dir),
            "-library",
            str(sim_fat),
            "-headers",
            str(headers_dir),
            "-library",
            str(macos_fat),
            "-headers",
            str(headers_dir),
            "-output",
            str(generated_dir / f"{lib_name}.xcframework"),
        ],
        cwd=root,
    )
    sim_fat.unlink(missing_ok=True)
    macos_fat.unlink(missing_ok=True)
    print(f"Generated Swift package with XCFramework in {rel(root, generated_dir)}")


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description="Generate crate-owned Auki SDK language bindings.")
    subparsers = parser.add_subparsers(dest="command", required=True)

    metadata_parser = subparsers.add_parser("metadata")
    metadata_parser.add_argument("crate")

    plan_parser = subparsers.add_parser("plan")
    plan_parser.add_argument("language", choices=sorted(LANGUAGES))
    plan_parser.add_argument("crate")

    generate_parser = subparsers.add_parser("generate")
    generate_parser.add_argument("language", choices=sorted(LANGUAGES))
    generate_parser.add_argument("crate")

    native_parser = subparsers.add_parser("python-native-libs")
    native_parser.add_argument("crate")
    native_parser.add_argument("targets", nargs="*")

    args = parser.parse_args(argv)
    root = repo_root()

    try:
        if args.command == "metadata":
            print(json.dumps(package_metadata(root, args.crate), sort_keys=True))
        elif args.command == "plan":
            print(json.dumps(plan(root, args.crate, args.language), sort_keys=True))
        elif args.command == "generate":
            if args.language == "python":
                generate_python(root, args.crate)
            elif args.language == "javascript":
                generate_javascript(root, args.crate)
            elif args.language == "swift":
                generate_swift(root, args.crate)
            elif args.language == "swift-xcframework":
                generate_swift_xcframework(root, args.crate)
        elif args.command == "python-native-libs":
            build_python_native_libraries(root, args.crate, args.targets)
    except BindingError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1

    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
